use super::*;

// ---- The fake Codex ----

/// What the fake Codex answers, and when.
pub(super) struct Answers {
    /// The hash of each request's audio, in order.
    pub(super) requests: Mutex<Vec<u64>>,
    /// Requests past this many wait until it is lifted.
    hold_after: Mutex<Option<usize>>,
    lifted: Condvar,
    delay: Duration,
}

pub(super) struct FakeCodex {
    pub(super) url: String,
    answers: Arc<Answers>,
}

impl FakeCodex {
    pub(super) fn start(delay: Duration, hold_after: Option<usize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transcribe", listener.local_addr().unwrap());
        let answers = Arc::new(Answers {
            requests: Mutex::default(),
            hold_after: Mutex::new(hold_after),
            lifted: Condvar::new(),
            delay,
        });
        let serving = Arc::clone(&answers);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let answers = Arc::clone(&serving);
                std::thread::spawn(move || answer(stream, &answers));
            }
        });
        Self { url, answers }
    }

    pub(super) fn requests(&self) -> Vec<u64> {
        self.answers.requests.lock().unwrap().clone()
    }

    pub(super) fn lift_hold(&self) {
        *self.answers.hold_after.lock().unwrap() = None;
        self.answers.lifted.notify_all();
    }
}

/// The body of an HTTP request, sized or chunked.
fn read_body(reader: &mut BufReader<TcpStream>) -> Option<Vec<u8>> {
    let (mut length, mut chunked) = (0, false);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end().to_ascii_lowercase();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("content-length:") {
            length = value.trim().parse().ok()?;
        }
        chunked |= line.starts_with("transfer-encoding:") && line.contains("chunked");
    }
    let mut body = Vec::new();
    if !chunked {
        body.resize(length, 0);
        reader.read_exact(&mut body).ok()?;
        return Some(body);
    }
    loop {
        let mut size = String::new();
        reader.read_line(&mut size).ok()?;
        let size = usize::from_str_radix(size.trim(), 16).ok()?;
        let mut data = vec![0; size + 2];
        reader.read_exact(&mut data).ok()?;
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(&data[..size]);
    }
}

pub(super) fn answer(stream: TcpStream, answers: &Answers) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let Some(body) = read_body(&mut reader) else {
        return;
    };
    let wav = uploaded_wav(&body).expect("a transcription request uploads a WAV");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    wav.hash(&mut hasher);
    let piece = hasher.finish();
    let number = {
        let mut requests = answers.requests.lock().unwrap();
        requests.push(piece);
        requests.len() - 1
    };
    drop(
        answers
            .lifted
            .wait_while(answers.hold_after.lock().unwrap(), |hold| {
                hold.is_some_and(|after| number >= after)
            })
            .unwrap(),
    );
    std::thread::sleep(answers.delay);
    let text = serde_json::json!({ "text": piece_text(piece, wav.len()) }).to_string();
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
}

/// The WAV a transcription request uploads, which identifies the piece. The
/// rest of the body differs at each request: reqwest draws a new multipart
/// boundary for each form.
fn uploaded_wav(body: &[u8]) -> Option<&[u8]> {
    let start = body.windows(4).position(|window| window == b"RIFF")?;
    let size: [u8; 4] = body.get(start + 4..start + 8)?.try_into().ok()?;
    body.get(start..start + 8 + u32::from_le_bytes(size) as usize)
}

/// About as many words as a track of half a dialogue holds in a piece of
/// `bytes` of 16-bit audio, tagged with the piece.
fn piece_text(piece: u64, bytes: usize) -> String {
    let words = bytes / 32_000 * 13 / 10;
    let mut text = format!("Piece {piece:016x}.");
    let mut count = 0;
    for (_, line) in LINES.iter().cycle().skip(piece as usize % LINES.len()) {
        if count >= words {
            break;
        }
        text.push(' ');
        text.push_str(line);
        count += line.split_whitespace().count();
    }
    text
}

/// The piece a transcribed chunk's text was the answer for.
pub(super) fn piece_of(text: &str) -> Option<u64> {
    let tag = text.strip_prefix("Piece ")?.get(..16)?;
    u64::from_str_radix(tag, 16).ok()
}

pub(super) fn auth_file(dir: &Path) -> PathBuf {
    let path = dir.join("auth.json");
    let auth = serde_json::json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "access_token": "soak-token",
            "refresh_token": "soak-refresh",
            "account_id": "soak-account"
        },
        "last_refresh": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(&path, auth.to_string()).unwrap();
    path
}

/// The dialogue, A then B in turn. Said three times at different rates, it
/// lasts about ten minutes.
pub(super) const LINES: &[(bool, &str)] = &[
    (false, "Bonjour à tous, merci d'être là ce matin pour faire le point sur le projet."),
    (true, "Bonjour, merci pour l'invitation. J'ai préparé les chiffres du dernier trimestre."),
    (false, "Parfait. Commençons par le calendrier, où en est la livraison de la nouvelle version ?"),
    (true, "La version est prête à quatre-vingts pour cent. Il reste les tests sur les anciens ordinateurs."),
    (false, "Combien de temps faut-il prévoir pour ces tests, à ton avis ?"),
    (true, "Je dirais deux semaines, si l'équipe de validation peut commencer lundi prochain."),
    (false, "D'accord. Et du côté du budget, est-ce qu'on reste dans l'enveloppe prévue ?"),
    (true, "Oui, nous avons dépensé un peu moins que prévu sur les serveurs ce trimestre."),
    (false, "Très bien. Est-ce qu'on peut réutiliser cette marge pour embaucher un stagiaire ?"),
    (true, "C'est possible, mais il faut d'abord valider la demande avec les ressources humaines."),
    (false, "Je m'en occupe cette semaine. Passons aux retours des clients sur la dernière version."),
    (true, "Les retours sont plutôt bons. Les utilisateurs apprécient surtout la recherche plus rapide."),
    (false, "Et les plaintes ? J'ai vu passer plusieurs messages sur la synchronisation."),
    (true, "Oui, la synchronisation échoue parfois quand la connexion est lente. Nous avons trouvé la cause."),
    (false, "Quelle est la cause exactement, et quand pouvons-nous livrer une correction ?"),
    (true, "Un délai trop court sur les requêtes. La correction partira avec la prochaine mise à jour."),
    (false, "Bien. Il faudra prévenir le support pour qu'ils puissent répondre aux clients."),
    (true, "Je leur enverrai une note cet après-midi avec les détails et les dates."),
    (false, "Merci. Parlons maintenant de la conférence du mois prochain à Lyon."),
    (true, "Nous avons un stand et deux présentations. Il manque encore quelqu'un pour la seconde."),
    (false, "Je peux faire la seconde présentation si tu m'envoies les diapositives avant vendredi."),
    (true, "Entendu, je te les envoie jeudi soir au plus tard, avec mes notes."),
    (false, "Est-ce qu'on a prévu des démonstrations sur le stand, ou seulement des affiches ?"),
    (true, "Deux ordinateurs de démonstration, et une tablette pour inscrire les visiteurs intéressés."),
    (false, "Il faudra vérifier que la connexion du salon permet de faire la démonstration en direct."),
    (true, "Bonne remarque. Je demanderai aux organisateurs le débit garanti sur les stands."),
    (false, "Autre sujet, la migration de la base de données. Où en sommes-nous ?"),
    (true, "La copie de test fonctionne. Nous voulons migrer la production un dimanche matin."),
    (false, "Combien de temps le service sera-t-il coupé pendant la migration ?"),
    (true, "Environ deux heures, et nous gardons l'ancienne base pendant un mois au cas où."),
    (false, "Ça me semble raisonnable. Il faudra prévenir les clients une semaine avant."),
    (true, "Je préparerai le message et je te le soumets avant de l'envoyer."),
    (false, "Parfait. Est-ce qu'il y a d'autres points que tu voulais aborder aujourd'hui ?"),
    (true, "Juste un, les congés d'été. Il faudrait que chacun donne ses dates avant la fin du mois."),
    (false, "Je relaierai la demande à l'équipe. On garde au moins deux personnes en août."),
    (true, "C'est noté. De mon côté, je serai absente les deux premières semaines de juillet."),
    (false, "Merci pour toutes ces informations. On se retrouve la semaine prochaine, même heure ?"),
    (true, "Oui, même heure. Bonne journée à tous, et merci pour cette réunion."),
];
