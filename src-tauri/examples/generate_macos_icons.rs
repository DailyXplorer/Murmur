#[cfg(target_os = "macos")]
mod macos {
    use anyhow::{anyhow, bail, Context, Result};
    use image::codecs::png::PngEncoder;
    use image::imageops::{self, FilterType};
    use image::{DynamicImage, ExtendedColorType, ImageEncoder, ImageReader, Rgba, RgbaImage};
    use std::env;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const SOURCE_DIMENSION: u32 = 1254;
    const PLATE_ANTIALIAS_SCALE: u32 = 4;

    #[derive(Clone, Copy)]
    struct AppIconSpec {
        canvas_size: u32,
        plate_inset: u32,
        plate_corner_radius: u32,
        mark_fit: u32,
    }

    #[derive(Clone, Copy)]
    struct TrayTemplateSpec {
        canvas_size: u32,
        alpha_threshold: u8,
        mark_fit: u32,
    }

    #[derive(Clone, Copy)]
    struct IconOutput {
        relative_path: &'static str,
    }

    #[derive(Clone, Copy)]
    struct OutputRegistry {
        app_icon: IconOutput,
        app_32: IconOutput,
        app_128: IconOutput,
        app_128_at_2x: IconOutput,
        app_icns: IconOutput,
        tray: IconOutput,
    }

    impl OutputRegistry {
        fn all(self) -> [IconOutput; 6] {
            [
                self.app_icon,
                self.app_32,
                self.app_128,
                self.app_128_at_2x,
                self.app_icns,
                self.tray,
            ]
        }
    }

    #[derive(Clone, Copy)]
    struct IconRecipe {
        app: AppIconSpec,
        tray: TrayTemplateSpec,
        outputs: OutputRegistry,
    }

    const ICON_RECIPE: IconRecipe = IconRecipe {
        app: AppIconSpec {
            canvas_size: 1024,
            plate_inset: 38,
            plate_corner_radius: 216,
            mark_fit: 782,
        },
        tray: TrayTemplateSpec {
            canvas_size: 64,
            alpha_threshold: 128,
            mark_fit: 46,
        },
        outputs: OutputRegistry {
            app_icon: IconOutput {
                relative_path: "src-tauri/icons/icon.png",
            },
            app_32: IconOutput {
                relative_path: "src-tauri/icons/32x32.png",
            },
            app_128: IconOutput {
                relative_path: "src-tauri/icons/128x128.png",
            },
            app_128_at_2x: IconOutput {
                relative_path: "src-tauri/icons/128x128@2x.png",
            },
            app_icns: IconOutput {
                relative_path: "src-tauri/icons/icon.icns",
            },
            tray: IconOutput {
                relative_path: "src-tauri/resources/tray.png",
            },
        },
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum GenerationMode {
        Write,
        Check,
    }

    struct PreparedReplacement {
        target: PathBuf,
        original: Option<Vec<u8>>,
        temporary: Option<tempfile::NamedTempFile>,
    }

    impl GenerationMode {
        fn from_args() -> Result<Self> {
            let args = env::args().skip(1).collect::<Vec<_>>();
            match args.as_slice() {
                [] => Ok(Self::Write),
                [flag] if flag == "--check" => Ok(Self::Check),
                _ => bail!("usage: generate_macos_icons [--check]"),
            }
        }
    }

    pub fn run() -> Result<()> {
        let mode = GenerationMode::from_args()?;
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("src-tauri manifest must have a repository parent")?;
        let source = repository_root.join("src-tauri/icons/source/murmur-mark.png");
        let source_mark = read_source_mark(&source)?;
        let stage = tempfile::tempdir().context("create temporary icon output directory")?;

        generate_all_outputs(ICON_RECIPE, &source_mark, stage.path())?;
        publish_or_check(ICON_RECIPE.outputs, repository_root, stage.path(), mode)
    }

    fn read_source_mark(path: &Path) -> Result<RgbaImage> {
        let source = ImageReader::open(path)
            .with_context(|| format!("open canonical source icon at {}", path.display()))?
            .decode()
            .context("decode canonical source icon")?;

        let DynamicImage::ImageRgba8(source) = source else {
            bail!("canonical source icon must be an RGBA PNG");
        };
        if source.dimensions() != (SOURCE_DIMENSION, SOURCE_DIMENSION) {
            bail!(
                "canonical source icon must be {SOURCE_DIMENSION}x{SOURCE_DIMENSION}, got {}x{}",
                source.width(),
                source.height()
            );
        }
        if !source.pixels().any(|pixel| pixel[3] != 0) {
            bail!("canonical source icon must contain nontransparent pixels");
        }

        Ok(source)
    }

    fn generate_all_outputs(recipe: IconRecipe, source: &RgbaImage, stage: &Path) -> Result<()> {
        let app_icon = generate_app_icon(recipe.app, source)?;
        write_png(&app_icon, &stage_path(stage, recipe.outputs.app_icon))?;
        write_png(
            &resize_square(&app_icon, 32),
            &stage_path(stage, recipe.outputs.app_32),
        )?;
        write_png(
            &resize_square(&app_icon, 128),
            &stage_path(stage, recipe.outputs.app_128),
        )?;
        write_png(
            &resize_square(&app_icon, 256),
            &stage_path(stage, recipe.outputs.app_128_at_2x),
        )?;
        build_icns(
            &app_icon,
            stage,
            &stage_path(stage, recipe.outputs.app_icns),
        )?;

        let tray = generate_tray_template(recipe.tray, source)?;
        write_png(&tray, &stage_path(stage, recipe.outputs.tray))?;

        Ok(())
    }

    fn generate_app_icon(spec: AppIconSpec, source: &RgbaImage) -> Result<RgbaImage> {
        let mut canvas = rounded_plate(spec);
        let mark = fit_within(&trim_alpha(source)?, spec.mark_fit);
        let x = ((spec.canvas_size - mark.width()) / 2) as i64;
        let y = ((spec.canvas_size - mark.height()) / 2) as i64;
        imageops::overlay(&mut canvas, &mark, x, y);
        Ok(canvas)
    }

    fn rounded_plate(spec: AppIconSpec) -> RgbaImage {
        let scale = PLATE_ANTIALIAS_SCALE;
        let canvas_size = spec.canvas_size * scale;
        let inset = spec.plate_inset * scale;
        let corner_radius = spec.plate_corner_radius * scale;
        let plate_end = canvas_size - inset;
        let mut plate = RgbaImage::from_pixel(canvas_size, canvas_size, Rgba([255, 255, 255, 0]));

        for y in inset..plate_end {
            for x in inset..plate_end {
                if point_is_inside_rounded_rect(x, y, inset, plate_end, corner_radius) {
                    plate.put_pixel(x, y, Rgba([255, 255, 255, 255]));
                }
            }
        }

        resize_square(&plate, spec.canvas_size)
    }

    fn point_is_inside_rounded_rect(
        x: u32,
        y: u32,
        start: u32,
        end: u32,
        corner_radius: u32,
    ) -> bool {
        let left_curve = start + corner_radius;
        let right_curve = end - corner_radius;
        let top_curve = start + corner_radius;
        let bottom_curve = end - corner_radius;
        let nearest_x = x.clamp(left_curve, right_curve);
        let nearest_y = y.clamp(top_curve, bottom_curve);
        let delta_x = x as i64 - nearest_x as i64;
        let delta_y = y as i64 - nearest_y as i64;
        delta_x * delta_x + delta_y * delta_y <= i64::from(corner_radius) * i64::from(corner_radius)
    }

    fn generate_tray_template(spec: TrayTemplateSpec, source: &RgbaImage) -> Result<RgbaImage> {
        let thresholded = source_from_alpha_threshold(source, spec.alpha_threshold);
        let mark = fit_within(&trim_alpha(&thresholded)?, spec.mark_fit);
        let mut canvas = RgbaImage::new(spec.canvas_size, spec.canvas_size);
        let x = (spec.canvas_size - mark.width()) / 2;
        let y = (spec.canvas_size - mark.height()) / 2;

        for (mark_x, mark_y, pixel) in mark.enumerate_pixels() {
            if pixel[3] != 0 {
                canvas.put_pixel(mark_x + x, mark_y + y, Rgba([0, 0, 0, pixel[3]]));
            }
        }

        Ok(canvas)
    }

    fn source_from_alpha_threshold(source: &RgbaImage, threshold: u8) -> RgbaImage {
        let mut mask = RgbaImage::new(source.width(), source.height());
        for (x, y, pixel) in source.enumerate_pixels() {
            let alpha = u8::from(pixel[3] >= threshold) * u8::MAX;
            mask.put_pixel(x, y, Rgba([0, 0, 0, alpha]));
        }
        mask
    }

    fn trim_alpha(image: &RgbaImage) -> Result<RgbaImage> {
        let mut min_x = image.width();
        let mut min_y = image.height();
        let mut max_x = 0;
        let mut max_y = 0;
        let mut found_visible_pixel = false;

        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel[3] == 0 {
                continue;
            }
            found_visible_pixel = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }

        if !found_visible_pixel {
            bail!("image has no nontransparent pixels to trim");
        }

        Ok(
            imageops::crop_imm(image, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
                .to_image(),
        )
    }

    fn fit_within(source: &RgbaImage, max_extent: u32) -> RgbaImage {
        let largest_dimension = source.width().max(source.height());
        let width = ((source.width() as f64 * f64::from(max_extent) / f64::from(largest_dimension))
            .round() as u32)
            .max(1);
        let height = ((source.height() as f64 * f64::from(max_extent)
            / f64::from(largest_dimension))
        .round() as u32)
            .max(1);
        resize_premultiplied(source, width, height)
    }

    fn resize_square(image: &RgbaImage, size: u32) -> RgbaImage {
        resize_premultiplied(image, size, size)
    }

    fn resize_premultiplied(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
        let mut premultiplied = image.clone();
        for pixel in premultiplied.pixels_mut() {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel.0[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }

        let mut resized = imageops::resize(&premultiplied, width, height, FilterType::Lanczos3);
        for pixel in resized.pixels_mut() {
            let alpha = u16::from(pixel[3]);
            if alpha == 0 {
                pixel.0 = [0, 0, 0, 0];
                continue;
            }
            for channel in &mut pixel.0[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        resized
    }

    fn build_icns(app_icon: &RgbaImage, stage: &Path, output: &Path) -> Result<()> {
        let iconset = stage.join("Murmur.iconset");
        fs::create_dir_all(&iconset).context("create temporary iconset")?;
        for (name, size) in [
            ("icon_16x16.png", 16),
            ("icon_16x16@2x.png", 32),
            ("icon_32x32.png", 32),
            ("icon_32x32@2x.png", 64),
            ("icon_128x128.png", 128),
            ("icon_128x128@2x.png", 256),
            ("icon_256x256.png", 256),
            ("icon_256x256@2x.png", 512),
            ("icon_512x512.png", 512),
            ("icon_512x512@2x.png", 1024),
        ] {
            write_png(&resize_square(app_icon, size), &iconset.join(name))?;
        }

        let status = Command::new("iconutil")
            .arg("-c")
            .arg("icns")
            .arg(&iconset)
            .arg("-o")
            .arg(output)
            .status()
            .context("run iconutil to build ICNS")?;
        if !status.success() {
            bail!("iconutil failed while building {}", output.display());
        }
        Ok(())
    }

    fn stage_path(stage: &Path, output: IconOutput) -> PathBuf {
        stage.join(output.relative_path)
    }

    fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgba8,
            )
            .context("encode PNG")?;
        Ok(bytes)
    }

    fn write_png(image: &RgbaImage, path: &Path) -> Result<()> {
        write_bytes(path, &encode_png(image)?)
    }

    fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
        let parent = path
            .parent()
            .context("generated output must have a parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("create generated output directory {}", parent.display()))?;
        fs::write(path, bytes).with_context(|| format!("write generated output {}", path.display()))
    }

    fn publish_or_check(
        outputs: OutputRegistry,
        repository_root: &Path,
        stage: &Path,
        mode: GenerationMode,
    ) -> Result<()> {
        let mut generated_outputs = Vec::new();
        let mut drifted_outputs = Vec::new();
        for output in outputs.all() {
            let generated = fs::read(stage_path(stage, output)).with_context(|| {
                format!("read staged generated output {}", output.relative_path)
            })?;
            let target = repository_root.join(output.relative_path);
            match mode {
                GenerationMode::Write => generated_outputs.push((target, generated)),
                GenerationMode::Check => match fs::read(&target) {
                    Ok(existing) if existing == generated => {}
                    Ok(_) | Err(_) => drifted_outputs.push(output.relative_path),
                },
            }
        }

        if mode == GenerationMode::Write {
            return publish_generated_outputs(generated_outputs);
        }

        if drifted_outputs.is_empty() {
            return Ok(());
        }
        Err(anyhow!(
            "generated icon outputs differ from the tracked files:\n{}\nrun icons:generate",
            drifted_outputs
                .into_iter()
                .map(|path| format!("  {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }

    fn publish_generated_outputs(outputs: Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
        let mut replacements = Vec::new();
        for (target, generated) in outputs {
            let original = match fs::read(&target) {
                Ok(existing) if existing == generated => continue,
                Ok(existing) => Some(existing),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("read generated output {}", target.display()))
                }
            };
            replacements.push(prepare_replacement(target, original, &generated)?);
        }

        publish_prepared_replacements(replacements)
    }

    fn publish_prepared_replacements(mut replacements: Vec<PreparedReplacement>) -> Result<()> {
        for index in 0..replacements.len() {
            let publish_result = {
                let replacement = &mut replacements[index];
                replacement
                    .temporary
                    .take()
                    .expect("prepared replacement should contain a temporary file")
                    .persist(&replacement.target)
            };
            if let Err(error) = publish_result {
                let target = replacements[index].target.display().to_string();
                let rollback_errors = rollback_replacements(&replacements[..index]);
                if rollback_errors.is_empty() {
                    return Err(error.error)
                        .with_context(|| format!("replace generated output {target}"));
                }
                bail!(
                    "replace generated output {target}: {}; rollback errors: {}",
                    error.error,
                    rollback_errors.join("; ")
                );
            }
        }
        Ok(())
    }

    fn prepare_replacement(
        target: PathBuf,
        original: Option<Vec<u8>>,
        bytes: &[u8],
    ) -> Result<PreparedReplacement> {
        let parent = target
            .parent()
            .context("generated output must have a parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("create generated output directory {}", parent.display()))?;

        let mut replacement = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("create replacement next to {}", target.display()))?;
        replacement
            .write_all(bytes)
            .with_context(|| format!("write replacement for {}", target.display()))?;
        replacement
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))
            .with_context(|| format!("set permissions for {}", target.display()))?;
        replacement
            .as_file_mut()
            .sync_all()
            .with_context(|| format!("flush replacement for {}", target.display()))?;
        Ok(PreparedReplacement {
            target,
            original,
            temporary: Some(replacement),
        })
    }

    fn rollback_replacements(replacements: &[PreparedReplacement]) -> Vec<String> {
        let mut errors = Vec::new();
        for replacement in replacements.iter().rev() {
            let result = match &replacement.original {
                Some(original) => prepare_replacement(replacement.target.clone(), None, original)
                    .and_then(|mut prepared| {
                        prepared
                            .temporary
                            .take()
                            .expect("rollback replacement should contain a temporary file")
                            .persist(&prepared.target)
                            .map(|_| ())
                            .map_err(|error| error.error.into())
                    }),
                None => match fs::remove_file(&replacement.target) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                },
            };
            if let Err(error) = result {
                errors.push(format!("{}: {error:#}", replacement.target.display()));
            }
        }
        errors
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        fn visible_bounds(image: &RgbaImage, minimum_alpha: u8) -> Option<(u32, u32, u32, u32)> {
            let mut min_x = image.width();
            let mut min_y = image.height();
            let mut max_x = 0;
            let mut max_y = 0;
            let mut found = false;

            for (x, y, pixel) in image.enumerate_pixels() {
                if pixel[3] < minimum_alpha {
                    continue;
                }
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
            found.then_some((min_x, min_y, max_x, max_y))
        }

        fn canonical_source() -> RgbaImage {
            read_source_mark(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/source/murmur-mark.png"),
            )
            .expect("canonical source should decode")
        }

        #[test]
        fn fit_within_preserves_aspect_ratio_and_limit() {
            let source = RgbaImage::from_pixel(200, 100, Rgba([0, 0, 0, 255]));
            let fitted = fit_within(&source, 46);
            assert_eq!(fitted.dimensions(), (46, 23));
        }

        #[test]
        fn tray_template_is_black_transparent_and_centered() {
            let mut source = RgbaImage::new(100, 100);
            source.put_pixel(20, 10, Rgba([255, 0, 0, 255]));
            source.put_pixel(79, 89, Rgba([255, 0, 0, 255]));
            let tray = generate_tray_template(
                TrayTemplateSpec {
                    canvas_size: 64,
                    alpha_threshold: 128,
                    mark_fit: 46,
                },
                &source,
            )
            .expect("source has visible pixels");

            assert!(tray.pixels().any(|pixel| pixel[3] == 0));
            assert!(tray
                .pixels()
                .filter(|pixel| pixel[3] != 0)
                .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0));
        }

        #[test]
        fn alpha_trim_removes_transparent_border() {
            let mut source = RgbaImage::new(5, 6);
            source.put_pixel(1, 2, Rgba([1, 2, 3, 4]));
            source.put_pixel(3, 4, Rgba([4, 5, 6, 7]));
            let trimmed = trim_alpha(&source).expect("source has visible pixels");
            assert_eq!(trimmed.dimensions(), (3, 3));
            assert_eq!(trimmed.get_pixel(0, 0), &Rgba([1, 2, 3, 4]));
        }

        #[test]
        fn rounded_plate_keeps_white_rgb_through_antialiasing() {
            let plate = rounded_plate(AppIconSpec {
                canvas_size: 64,
                plate_inset: 4,
                plate_corner_radius: 12,
                mark_fit: 48,
            });
            let edge_pixels = plate
                .pixels()
                .filter(|pixel| pixel[3] > 0 && pixel[3] < 255)
                .collect::<Vec<_>>();

            assert!(!edge_pixels.is_empty());
            assert!(edge_pixels
                .into_iter()
                .all(|pixel| pixel.0[..3] == [255, 255, 255]));
        }

        #[test]
        fn premultiplied_resize_avoids_transparent_edge_halos() {
            let mut source = RgbaImage::new(2, 2);
            source.put_pixel(0, 0, Rgba([255, 0, 128, 255]));
            let resized = resize_premultiplied(&source, 16, 16);
            let edge_pixels = resized
                .pixels()
                .filter(|pixel| pixel[3] >= 32 && pixel[3] < 255)
                .collect::<Vec<_>>();

            assert!(!edge_pixels.is_empty());
            assert!(edge_pixels.into_iter().all(|pixel| {
                pixel[0] >= 250 && pixel[1] <= 2 && (i16::from(pixel[2]) - 128).abs() <= 4
            }));
        }

        #[test]
        fn production_tray_template_keeps_the_m_at_menu_bar_size() {
            let tray = generate_tray_template(ICON_RECIPE.tray, &canonical_source())
                .expect("canonical source should produce a tray template");
            assert_eq!(visible_bounds(&tray, 1), Some((9, 13, 54, 50)));

            let rendered = resize_square(&tray, 18);
            assert_eq!(visible_bounds(&rendered, 16), Some((2, 3, 15, 14)));
            assert!(rendered.get_pixel(4, 4)[3] > 200);
            assert!(rendered.get_pixel(13, 4)[3] > 200);
            assert!(rendered.get_pixel(9, 3)[3] < 32);
            assert!(rendered.get_pixel(9, 8)[3] > 240);
            assert!(rendered.get_pixel(4, 14)[3] > 32);
            assert!(rendered.get_pixel(9, 14)[3] < 32);
            assert!(rendered.get_pixel(13, 14)[3] > 32);
        }

        #[test]
        fn publishing_skips_unchanged_files_and_replaces_changed_files() {
            let directory = tempfile::tempdir().expect("temporary directory should be created");
            let target = directory.path().join("icon.png");
            fs::write(&target, b"original").expect("fixture should be written");
            let original_inode = fs::metadata(&target)
                .expect("fixture metadata should be readable")
                .ino();

            publish_generated_outputs(vec![(target.clone(), b"original".to_vec())])
                .expect("unchanged output should be skipped");
            assert_eq!(
                fs::metadata(&target)
                    .expect("unchanged metadata should be readable")
                    .ino(),
                original_inode
            );

            publish_generated_outputs(vec![(target.clone(), b"replacement".to_vec())])
                .expect("changed output should be replaced");
            assert_eq!(
                fs::read(&target).expect("replacement should be readable"),
                b"replacement"
            );
            assert_eq!(
                fs::metadata(&target)
                    .expect("replacement metadata should be readable")
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
        }

        #[test]
        fn publishing_rolls_back_outputs_after_a_later_failure() {
            let directory = tempfile::tempdir().expect("temporary directory should be created");
            let first = directory.path().join("first.png");
            let second = directory.path().join("second.png");
            fs::write(&first, b"first original").expect("first fixture should be written");
            fs::write(&second, b"second original").expect("second fixture should be written");

            let replacements = vec![
                prepare_replacement(
                    first.clone(),
                    Some(b"first original".to_vec()),
                    b"first replacement",
                )
                .expect("first replacement should be prepared"),
                prepare_replacement(
                    second.clone(),
                    Some(b"second original".to_vec()),
                    b"second replacement",
                )
                .expect("second replacement should be prepared"),
            ];
            fs::remove_file(&second).expect("second fixture should be removed");
            fs::create_dir(&second).expect("second target should become an invalid directory");

            assert!(publish_prepared_replacements(replacements).is_err());
            assert_eq!(
                fs::read(&first).expect("first output should be restored"),
                b"first original"
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = macos::run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("generate_macos_icons is only available on macOS");
    std::process::exit(1);
}
