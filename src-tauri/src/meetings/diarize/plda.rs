//! The PLDA transform that turns a 256-value speaker embedding into the 128
//! values the VBx clustering works on, and the `psi` vector that clustering
//! needs next to them.
//!
//! The spec is the `PldaRho` Core ML graph (`mil/plda.mil`), which
//! `PLDATransform.swift` only calls: no maths is left in the Swift, whose
//! `transform` batches the rows by 32 for the model's shape limit and widens
//! the answer to `f64`. The graph, per embedding `e`:
//!
//! 1. `x = e - mean1`, `u = x / sqrt(max(|x|^2, 1e-12))`
//! 2. `p = (u . lda) * sqrt(256) - mean2`
//! 3. `v = p / sqrt(max(|p|^2, 1e-12)) * sqrt(128)`
//! 4. `rho = ((v - mu) . tr^T) * sqrt(psi)`
//!
//! `lda` is `[256, 128]` and `tr` `[128, 128]` as the JSON files hold them. In
//! the graph `lda` is stored transposed (`transpose_0`, `[128, 256]`) and `tr`
//! as is (`transform_plda_tr`), both fed to `linear`, so step 2 is `u @ lda`
//! and step 4 is `tr @ (v - mu)`. Both `linear` calls share one bias constant
//! (`var_23_bias_0`) which the graph builder gives to a layer without bias:
//! zeros.
//!
//! The graph runs in f32. Here the sums run in f64, which differs from it by
//! rounding only.
//!
//! The dimensions come from the files (`lda` is `[d_in, d_out]`), so a test
//! can use a 4-to-2 transform; `Extractor` requires 256 to 128.

use serde_json::Value;

/// `plda.mil`, `var_4` (0x1.197998p-40 is 1e-12): the floor of a squared norm
/// before the square root, so a zero vector divides by 1e-6 and stays finite.
const NORM_FLOOR: f64 = 1.0e-12;

/// The constants of one PLDA model, from the two JSON files that ship next to
/// the Core ML models (`plda-parameters.json`, `xvector-transform.json`).
pub struct Plda {
    input_dim: usize,
    output_dim: usize,
    mean1: Vec<f64>,
    /// `[input_dim, output_dim]`, row major.
    lda: Vec<f64>,
    mean2: Vec<f64>,
    mu: Vec<f64>,
    /// `[output_dim, output_dim]`, row major.
    tr: Vec<f64>,
    psi: Vec<f64>,
    sqrt_psi: Vec<f64>,
    lda_scale: f64,
    dim_scale: f64,
}

impl Plda {
    /// Reads the two JSON documents. Every tensor is a base64 string of
    /// little-endian f32 values, row major, with its shape beside it.
    /// `plda_parameters` holds `mu`, `tr` and `psi`; `xvector_transform` holds
    /// `mean1`, `mean2` and `lda`.
    pub fn from_json(plda_parameters: &str, xvector_transform: &str) -> Result<Plda, &'static str> {
        let plda_doc: Value =
            serde_json::from_str(plda_parameters).map_err(|_| "plda: parameters are not JSON")?;
        let xvec_doc: Value =
            serde_json::from_str(xvector_transform).map_err(|_| "plda: transform is not JSON")?;

        let lda_shape: Vec<usize> = tensor_shape(&xvec_doc, "lda")?;
        let (input_dim, output_dim): (usize, usize) = match lda_shape.as_slice() {
            [rows, columns] if *rows > 0 && *columns > 0 => (*rows, *columns),
            _ => return Err("plda: lda is not a matrix"),
        };
        let lda: Vec<f64> = read_tensor(&xvec_doc, "lda", &[input_dim, output_dim])?;
        let mean1: Vec<f64> = read_tensor(&xvec_doc, "mean1", &[input_dim])?;
        let mean2: Vec<f64> = read_tensor(&xvec_doc, "mean2", &[output_dim])?;
        let mu: Vec<f64> = read_tensor(&plda_doc, "mu", &[output_dim])?;
        let tr: Vec<f64> = read_tensor(&plda_doc, "tr", &[output_dim, output_dim])?;
        let psi: Vec<f64> = read_tensor(&plda_doc, "psi", &[output_dim])?;
        if psi.iter().any(|&value| value < 0.0) {
            return Err("plda: psi has a negative value");
        }
        let sqrt_psi: Vec<f64> = psi.iter().map(|&value| value.sqrt()).collect();

        Ok(Plda {
            input_dim,
            output_dim,
            mean1,
            lda,
            mean2,
            mu,
            tr,
            psi,
            sqrt_psi,
            // plda.mil `transform_lda_scale` (16) and `transform_lda_dim_scale`
            // (0x1.6a09e6p+3, 11.3137): the square roots of the two sizes.
            lda_scale: (input_dim as f64).sqrt(),
            dim_scale: (output_dim as f64).sqrt(),
        })
    }

    /// Size of an embedding this transform takes (256).
    pub fn input_dim(&self) -> usize {
        self.input_dim
    }

    /// Size of a transformed embedding (128).
    pub fn output_dim(&self) -> usize {
        self.output_dim
    }

    /// The `psi` of the PLDA model, `output_dim` values. `PLDATransform.phiParameters`
    /// (line 22) is this vector, and `VBxClustering.swift` line 72 reads nothing
    /// else from the transform. VBx clamps each value at 1e-12 itself (line 240).
    pub fn psi(&self) -> &[f64] {
        &self.psi
    }

    /// The PLDA features (`rho`) of each embedding, in order. A row that does
    /// not hold `input_dim` values gives an empty row (the Swift throws; this
    /// stage has no caller that can pass one).
    pub fn transform(&self, embeddings: &[Vec<f32>]) -> Vec<Vec<f64>> {
        embeddings
            .iter()
            .map(|embedding| self.transform_one(embedding))
            .collect()
    }

    fn transform_one(&self, embedding: &[f32]) -> Vec<f64> {
        if embedding.len() != self.input_dim {
            return Vec::new();
        }
        let centred: Vec<f64> = embedding
            .iter()
            .zip(&self.mean1)
            .map(|(&value, &mean)| f64::from(value) - mean)
            .collect();
        let unit: Vec<f64> = scale_to_unit_norm(&centred, 1.0);

        let mut projected: Vec<f64> = vec![0.0; self.output_dim];
        for (&component, row) in unit.iter().zip(self.lda.chunks_exact(self.output_dim)) {
            for (sum, &weight) in projected.iter_mut().zip(row) {
                *sum += component * weight;
            }
        }
        for (value, &mean) in projected.iter_mut().zip(&self.mean2) {
            *value = *value * self.lda_scale - mean;
        }

        let mut features: Vec<f64> = scale_to_unit_norm(&projected, self.dim_scale);
        for (value, &mean) in features.iter_mut().zip(&self.mu) {
            *value -= mean;
        }

        self.tr
            .chunks_exact(self.output_dim)
            .zip(&self.sqrt_psi)
            .map(|(row, &root)| {
                let dot: f64 = row
                    .iter()
                    .zip(&features)
                    .map(|(&weight, &value)| weight * value)
                    .sum();
                dot * root
            })
            .collect()
    }
}

/// `values * scale / sqrt(max(|values|^2, NORM_FLOOR))`.
fn scale_to_unit_norm(values: &[f64], scale: f64) -> Vec<f64> {
    let squares: f64 = values.iter().map(|&value| value * value).sum();
    let factor: f64 = scale / squares.max(NORM_FLOOR).sqrt();
    values.iter().map(|&value| value * factor).collect()
}

fn tensor<'a>(doc: &'a Value, name: &str) -> Result<&'a Value, &'static str> {
    doc.get("tensors")
        .and_then(|tensors| tensors.get(name))
        .ok_or("plda: a tensor is missing")
}

fn tensor_shape(doc: &Value, name: &str) -> Result<Vec<usize>, &'static str> {
    let shape = tensor(doc, name)?
        .get("shape")
        .and_then(Value::as_array)
        .ok_or("plda: a tensor has no shape")?;
    shape
        .iter()
        .map(|dimension| {
            dimension
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("plda: a tensor shape is not made of sizes")
        })
        .collect()
}

/// One tensor as f64, checked against the shape the model needs.
fn read_tensor(doc: &Value, name: &str, expected: &[usize]) -> Result<Vec<f64>, &'static str> {
    let entry = tensor(doc, name)?;
    if tensor_shape(doc, name)? != expected {
        return Err("plda: a tensor has the wrong shape");
    }
    if entry.get("dtype").and_then(Value::as_str) != Some("float32") {
        return Err("plda: a tensor is not float32");
    }
    if let Some(layout) = entry.get("layout") {
        if layout.as_str() != Some("row-major") {
            return Err("plda: a tensor is not row-major");
        }
    }
    let text: &str = entry
        .get("data_base64")
        .and_then(Value::as_str)
        .ok_or("plda: a tensor has no data")?;
    let bytes: Vec<u8> = decode_base64(text)?;
    let count: usize = expected.iter().product();
    if bytes.len() != count * 4 {
        return Err("plda: a tensor has the wrong byte length");
    }
    let values: Vec<f64> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f64::from(f32::from_le_bytes(*chunk)))
        .collect();
    if values.iter().any(|value| !value.is_finite()) {
        return Err("plda: a tensor holds a value that is not finite");
    }
    Ok(values)
}

/// Standard base64 with `=` padding. The crate list for this stage has no
/// base64 decoder, and the files are the only place the format is read.
fn decode_base64(text: &str) -> Result<Vec<u8>, &'static str> {
    fn sextet(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let raw: &[u8] = text.as_bytes();
    let padding: usize = raw.iter().rev().take_while(|&&byte| byte == b'=').count();
    if padding > 2 || !raw.len().is_multiple_of(4) {
        return Err("plda: bad base64");
    }
    let data: &[u8] = &raw[..raw.len() - padding];
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 3 / 4);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        let value: u32 = sextet(byte).ok_or("plda: bad base64")?;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((accumulator >> bits) & 0xff) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn base64_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
            let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((triple >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((triple >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[((triple >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(triple & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    fn tensor_json(shape: &[usize], values: &[f32]) -> String {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
        format!(
            "{{\"shape\":[{}],\"dtype\":\"float32\",\"layout\":\"row-major\",\"data_base64\":\"{}\"}}",
            dims.join(","),
            base64_encode(&bytes)
        )
    }

    /// The constants of a transform, to be written as the two documents.
    pub(crate) struct Constants {
        pub mean1: Vec<f32>,
        pub lda: Vec<f32>,
        pub mean2: Vec<f32>,
        pub mu: Vec<f32>,
        pub tr: Vec<f32>,
        pub psi: Vec<f32>,
    }

    /// A 256-to-128 transform that keeps the first 128 coordinates: `lda` and
    /// `tr` are identity blocks, every mean is 0 and `psi` is 1. For the tests
    /// of the stage that calls the transform.
    pub(crate) fn keep_first_128() -> Plda {
        let mut lda: Vec<f32> = vec![0.0; 256 * 128];
        let mut tr: Vec<f32> = vec![0.0; 128 * 128];
        for index in 0..128 {
            lda[index * 128 + index] = 1.0;
            tr[index * 128 + index] = 1.0;
        }
        let constants = Constants {
            mean1: vec![0.0; 256],
            lda,
            mean2: vec![0.0; 128],
            mu: vec![0.0; 128],
            tr,
            psi: vec![1.0; 128],
        };
        let (plda, xvec) = documents(&constants);
        Plda::from_json(&plda, &xvec).expect("synthetic constants load")
    }

    /// The two documents for a transform with these constants, in the shape
    /// of the real files.
    fn documents(c: &Constants) -> (String, String) {
        let d_in = c.mean1.len();
        let d_out = c.mean2.len();
        let plda = format!(
            "{{\"name\":\"plda_parameters\",\"tensors\":{{\"mu\":{},\"tr\":{},\"psi\":{}}}}}",
            tensor_json(&[d_out], &c.mu),
            tensor_json(&[d_out, d_out], &c.tr),
            tensor_json(&[d_out], &c.psi)
        );
        let xvec = format!(
            "{{\"name\":\"xvec_transform\",\"tensors\":{{\"mean1\":{},\"mean2\":{},\"lda\":{}}}}}",
            tensor_json(&[d_in], &c.mean1),
            tensor_json(&[d_out], &c.mean2),
            tensor_json(&[d_in, d_out], &c.lda)
        );
        (plda, xvec)
    }

    /// 4 to 2, every number chosen so the answer can be followed by hand.
    fn tiny() -> Constants {
        Constants {
            mean1: vec![1.0, -1.0, 0.0, 0.0],
            lda: vec![1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
            mean2: vec![1.0, 0.0],
            mu: vec![0.25, -0.5],
            tr: vec![1.0, 2.0, 0.0, 1.0],
            psi: vec![4.0, 0.25],
        }
    }

    pub(crate) fn tiny_plda() -> Plda {
        let (plda, xvec) = documents(&tiny());
        Plda::from_json(&plda, &xvec).expect("tiny constants load")
    }

    #[test]
    fn tiny_transform_matches_the_hand_computed_answer() {
        // e = [2, 0, 1, 1].
        // 1. x = e - mean1 = [1, 1, 1, 1], |x| = 2, u = [.5, .5, .5, .5].
        // 2. u @ lda = [.5 + .5, .5 + .5] = [1, 1]; times sqrt(4) = 2 gives
        //    [2, 2]; minus mean2 [1, 0] gives p = [1, 2].
        // 3. |p| = sqrt(5); v = p / sqrt(5) * sqrt(2) = [sqrt(.4), 2 sqrt(.4)]
        //    = [0.6324555, 1.2649111].
        // 4. c = v - mu = [0.3824555, 1.7649111]; tr rows [1, 2] and [0, 1]
        //    give [0.3824555 + 3.5298222, 1.7649111] = [3.9122777, 1.7649111];
        //    times sqrt(psi) = [2, .5] gives rho = [7.8245554, 0.8824555].
        let rho = tiny_plda().transform(&[vec![2.0, 0.0, 1.0, 1.0]]);
        assert_eq!(rho.len(), 1);
        assert_eq!(rho[0].len(), 2);
        assert!((rho[0][0] - 7.8245554).abs() < 1e-6, "{:?}", rho[0]);
        assert!((rho[0][1] - 0.8824555).abs() < 1e-6, "{:?}", rho[0]);
    }

    #[test]
    fn tr_is_applied_as_rows_times_vector() {
        // With tr = [[1, 2], [0, 1]] (not symmetric) the first output mixes
        // both inputs and the second only the second: the transposed reading
        // would swap that.
        let mut c = tiny();
        c.mu = vec![0.0, 0.0];
        c.psi = vec![1.0, 1.0];
        let (plda, xvec) = documents(&c);
        let model = Plda::from_json(&plda, &xvec).expect("loads");
        // p = [1, 2] from the case above, v = [sqrt(.4), 2 sqrt(.4)].
        let rho = model.transform(&[vec![2.0, 0.0, 1.0, 1.0]]);
        let a = 0.4f64.sqrt();
        assert!((rho[0][0] - (a + 2.0 * 2.0 * a)).abs() < 1e-6);
        assert!((rho[0][1] - 2.0 * a).abs() < 1e-6);
    }

    #[test]
    fn psi_is_exposed_as_stored() {
        let model = tiny_plda();
        assert_eq!(model.psi(), &[4.0, 0.25]);
        assert_eq!(model.input_dim(), 4);
        assert_eq!(model.output_dim(), 2);
    }

    #[test]
    fn zero_after_centring_stays_finite() {
        // e equals mean1: x = 0, and the floor keeps 0 / sqrt(1e-12) at 0.
        let rho = tiny_plda().transform(&[vec![1.0, -1.0, 0.0, 0.0]]);
        assert!(rho[0].iter().all(|value| value.is_finite()));
    }

    #[test]
    fn transform_keeps_order_and_empty_input() {
        let model = tiny_plda();
        assert!(model.transform(&[]).is_empty());
        let rows = model.transform(&[vec![2.0, 0.0, 1.0, 1.0], vec![0.0, 0.0, 3.0, 0.0]]);
        let single = model.transform(&[vec![0.0, 0.0, 3.0, 0.0]]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], single[0]);
    }

    #[test]
    fn a_row_of_the_wrong_size_gives_an_empty_row() {
        let rows = tiny_plda().transform(&[vec![1.0, 2.0, 3.0]]);
        assert_eq!(rows, vec![Vec::<f64>::new()]);
    }

    #[test]
    fn malformed_documents_are_refused() {
        let (plda, xvec) = documents(&tiny());
        assert!(Plda::from_json("not json", &xvec).is_err());
        assert!(Plda::from_json(&plda, "{}").is_err());
        // a shape that disagrees with lda
        let wrong = plda.replace("\"shape\":[2,2]", "\"shape\":[3,3]");
        assert_eq!(
            Plda::from_json(&wrong, &xvec).err(),
            Some("plda: a tensor has the wrong shape")
        );
        let dtype = xvec.replacen("float32", "float64", 1);
        assert_eq!(
            Plda::from_json(&plda, &dtype).err(),
            Some("plda: a tensor is not float32")
        );
        let layout = plda.replacen("row-major", "column-major", 1);
        assert_eq!(
            Plda::from_json(&layout, &xvec).err(),
            Some("plda: a tensor is not row-major")
        );
        // a shape of 2 values whose data holds only 1
        let mu = tiny().mu;
        let short = plda.replacen(
            &tensor_json(&[2], &mu),
            &tensor_json(&[1], &mu[..1]).replacen("[1]", "[2]", 1),
            1,
        );
        assert_ne!(short, plda);
        assert_eq!(
            Plda::from_json(&short, &xvec).err(),
            Some("plda: a tensor has the wrong byte length")
        );
        // a character outside the base64 alphabet
        let bad = xvec.replacen("data_base64\":\"", "data_base64\":\"!", 1);
        assert!(Plda::from_json(&plda, &bad).is_err());
    }

    #[test]
    fn a_negative_psi_or_a_non_finite_value_is_refused() {
        let mut c = tiny();
        c.psi = vec![4.0, -0.25];
        let (plda, xvec) = documents(&c);
        assert!(Plda::from_json(&plda, &xvec).is_err());
        let mut c = tiny();
        c.mu = vec![f32::NAN, 0.0];
        let (plda, xvec) = documents(&c);
        assert!(Plda::from_json(&plda, &xvec).is_err());
    }

    #[test]
    fn base64_round_trips_every_padding() {
        for len in 0..9usize {
            let bytes: Vec<u8> = (0..len as u8).map(|b| b.wrapping_mul(37)).collect();
            assert_eq!(
                decode_base64(&base64_encode(&bytes)).as_deref(),
                Ok(&bytes[..])
            );
        }
        assert!(decode_base64("abc").is_err());
        assert!(decode_base64("ab=c").is_err());
    }

    // --- the real constants -------------------------------------------------

    /// Reads the two files from `MURMUR_PLDA_DIR`.
    fn real_model() -> (Plda, Value, Value) {
        let dir = std::env::var("MURMUR_PLDA_DIR").expect("set MURMUR_PLDA_DIR");
        let plda = std::fs::read_to_string(format!("{dir}/plda-parameters.json")).expect("plda");
        let xvec = std::fs::read_to_string(format!("{dir}/xvector-transform.json")).expect("xvec");
        let model = Plda::from_json(&plda, &xvec).expect("real files load");
        (
            model,
            serde_json::from_str(&plda).expect("json"),
            serde_json::from_str(&xvec).expect("json"),
        )
    }

    /// The same maths written the long way: dense matrices, plain loops over
    /// indices, f64 throughout, no helper from the code under test except the
    /// tensor reader.
    fn reference(plda: &Value, xvec: &Value, embedding: &[f32]) -> Vec<f64> {
        let mean1 = read_tensor(xvec, "mean1", &[256]).expect("mean1");
        let lda = read_tensor(xvec, "lda", &[256, 128]).expect("lda");
        let mean2 = read_tensor(xvec, "mean2", &[128]).expect("mean2");
        let mu = read_tensor(plda, "mu", &[128]).expect("mu");
        let tr = read_tensor(plda, "tr", &[128, 128]).expect("tr");
        let psi = read_tensor(plda, "psi", &[128]).expect("psi");

        let mut norm_sq = 0.0;
        for i in 0..256 {
            let x = f64::from(embedding[i]) - mean1[i];
            norm_sq += x * x;
        }
        let norm1 = if norm_sq > 1e-12 {
            norm_sq.sqrt()
        } else {
            1e-6
        };
        let mut projected = [0.0f64; 128];
        for j in 0..128 {
            let mut acc = 0.0;
            for i in 0..256 {
                acc += (f64::from(embedding[i]) - mean1[i]) / norm1 * lda[i * 128 + j];
            }
            projected[j] = acc * 16.0 - mean2[j];
        }
        let mut norm_sq2 = 0.0;
        for value in projected {
            norm_sq2 += value * value;
        }
        let norm2 = if norm_sq2 > 1e-12 {
            norm_sq2.sqrt()
        } else {
            1e-6
        };
        let mut centred = [0.0f64; 128];
        for j in 0..128 {
            centred[j] = projected[j] / norm2 * 128f64.sqrt() - mu[j];
        }
        let mut rho = vec![0.0f64; 128];
        for o in 0..128 {
            let mut acc = 0.0;
            for i in 0..128 {
                acc += tr[o * 128 + i] * centred[i];
            }
            rho[o] = acc * psi[o].sqrt();
        }
        rho
    }

    fn probe_inputs() -> Vec<Vec<f32>> {
        let mut unit_0 = vec![0.0f32; 256];
        unit_0[0] = 1.0;
        let mut unit_100 = vec![0.0f32; 256];
        unit_100[100] = 1.0;
        let all_ones: Vec<f32> = vec![1.0 / 16.0; 256];
        let ramp: Vec<f32> = (0..256).map(|i| (i as f32 - 127.5) / 256.0).collect();
        vec![unit_0, unit_100, all_ones, ramp]
    }

    #[test]
    #[ignore = "needs the PLDA json files"]
    fn real_files_match_the_naive_reference() {
        let (model, plda_doc, xvec_doc) = real_model();
        assert_eq!(model.input_dim(), 256);
        assert_eq!(model.output_dim(), 128);
        assert_eq!(model.psi().len(), 128);
        assert!(model
            .psi()
            .iter()
            .all(|&value| value > 0.0 && value.is_finite()));
        let inputs = probe_inputs();
        let got = model.transform(&inputs);
        for (input, row) in inputs.iter().zip(&got) {
            let want = reference(&plda_doc, &xvec_doc, input);
            assert_eq!(row.len(), 128);
            for (a, b) in row.iter().zip(&want) {
                assert!((a - b).abs() < 1e-9 * (1.0 + b.abs()), "{a} vs {b}");
            }
        }
    }

    #[test]
    #[ignore = "needs the PLDA json files"]
    fn real_files_match_precomputed_values() {
        // Printed once by `reference` above on the shipped files and pasted.
        let (model, _, _) = real_model();
        let got = model.transform(&probe_inputs());
        for (row, column, want) in PINNED {
            assert!(
                (got[row][column] - want).abs() < 1e-6 * (1.0 + want.abs()),
                "row {row} column {column}: {} vs {want}",
                got[row][column]
            );
        }
    }

    #[test]
    #[ignore = "prints the pinned values"]
    fn print_reference_values() {
        let (_, plda_doc, xvec_doc) = real_model();
        for (row, input) in probe_inputs().iter().enumerate() {
            let rho = reference(&plda_doc, &xvec_doc, input);
            for column in [0usize, 1, 64, 127] {
                println!("({row}, {column}, {:.9}),", rho[column]);
            }
        }
    }

    /// (input row of `probe_inputs`, output column, value).
    const PINNED: [(usize, usize, f64); 5] = [
        (0, 0, 2.019860871),
        (1, 0, 8.453389844),
        (2, 1, 0.012748211),
        (2, 64, -1.698537322),
        (3, 127, -1.200677445),
    ];
}
