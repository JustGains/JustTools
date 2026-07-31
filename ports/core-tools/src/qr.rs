use crate::common::{
    absolute_lexical, atomic_write, confirm, display_path, parse_cli, read_stdin, stdin_is_terminal,
};
use anyhow::{Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use image::ImageEncoder;
use image::codecs::png::PngEncoder;
use qrcode::{Color, EcLevel, QrCode};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ErrorLevel {
    L,
    M,
    Q,
    H,
}

impl ErrorLevel {
    fn qrcode(self) -> EcLevel {
        match self {
            Self::L => EcLevel::L,
            Self::M => EcLevel::M,
            Self::Q => EcLevel::Q,
            Self::H => EcLevel::H,
        }
    }
}

impl std::fmt::Display for ErrorLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}", self)
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "justqr",
    about = "Generate a ready-to-scan QR code.",
    after_help = "Defaults: qr.png, 1024 px, error correction Q, four-module quiet zone,\nblack foreground, and white background. Multiple text arguments are joined with spaces."
)]
struct Cli {
    /// Output path (default: qr.png or qr.svg).
    #[arg(short = 'o', long, value_name = "FILE", conflicts_with = "terminal")]
    output: Option<PathBuf>,

    /// Generate scalable SVG instead of PNG.
    #[arg(long, conflicts_with = "terminal")]
    svg: bool,

    /// Draw a compact QR code in the terminal.
    #[arg(short = 't', long, conflicts_with_all = ["svg", "output"])]
    terminal: bool,

    /// PNG width, 64-4096 (default: 1024).
    #[arg(short = 'w', long, default_value_t = 1024, value_parser = clap::value_parser!(u32).range(64..=4096))]
    width: u32,

    /// Error correction L, M, Q, or H (default: Q).
    #[arg(short = 'e', long = "error", default_value_t = ErrorLevel::Q, ignore_case = true)]
    error: ErrorLevel,

    /// Quiet-zone modules, 0-20 (default: 4).
    #[arg(short = 'm', long, default_value_t = 4, value_parser = clap::value_parser!(u32).range(0..=20))]
    margin: u32,

    /// Foreground 6- or 8-digit hex color.
    #[arg(long, default_value = "#000000")]
    dark: String,

    /// Background 6- or 8-digit hex color.
    #[arg(long, default_value = "#ffffff")]
    light: String,

    /// Replace an existing output without asking.
    #[arg(short = 'y', long)]
    yes: bool,

    /// Show the resolved format and output.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Text to encode. Multiple arguments are joined with spaces.
    #[arg(value_name = "TEXT")]
    text: Vec<String>,
}

pub fn run() -> Result<()> {
    let Some(options) = parse_cli::<Cli>()? else {
        return Ok(());
    };
    run_with(options)
}

fn run_with(mut options: Cli) -> Result<()> {
    let mut text = options.text.join(" ");
    if text.is_empty() && !stdin_is_terminal() {
        text = read_stdin()?;
        if text.ends_with("\r\n") {
            text.truncate(text.len() - 2);
        } else if text.ends_with('\n') {
            text.pop();
        }
    }
    if text.is_empty() {
        bail!("text is required");
    }

    if options
        .output
        .as_ref()
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        options.svg = true;
    }
    if let Some(output) = options.output.as_ref() {
        let valid = output
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("png") || extension.eq_ignore_ascii_case("svg")
            });
        if !valid {
            bail!("output must end in .png or .svg");
        }
    }

    let format = if options.terminal {
        "terminal"
    } else if options.svg {
        "svg"
    } else {
        "png"
    };
    let output = if options.terminal {
        None
    } else {
        Some(absolute_lexical(options.output.unwrap_or_else(|| {
            PathBuf::from(if options.svg { "qr.svg" } else { "qr.png" })
        }))?)
    };

    if options.dry_run {
        if let Some(output) = output.as_ref() {
            println!(
                "justqr: dry run — {format} -> {}, {} text byte(s)",
                display_path(output),
                text.len()
            );
        } else {
            println!("justqr: dry run — terminal, {} text byte(s)", text.len());
        }
        return Ok(());
    }

    let dark = parse_color(&options.dark)?;
    let light = parse_color(&options.light)?;
    let code = QrCode::with_error_correction_level(text.as_bytes(), options.error.qrcode())
        .map_err(|error| anyhow!("could not encode QR code: {error}"))?;
    if options.terminal {
        print!("{}", terminal_qr(&code, options.margin));
        return Ok(());
    }

    let output = output.expect("non-terminal output was resolved");
    if output.exists()
        && !options.yes
        && !confirm(&format!("justqr: replace {}", display_path(&output)))?
    {
        bail!("cancelled");
    }
    let bytes = if options.svg {
        svg_qr(&code, options.margin, &options.dark, &options.light).into_bytes()
    } else {
        png_qr(&code, options.margin, options.width, dark, light)?
    };
    atomic_write(&output, &bytes)?;
    println!(
        "justqr: wrote {} ({}{}, error {})",
        display_path(&output),
        format.to_ascii_uppercase(),
        if options.svg {
            String::new()
        } else {
            format!(", {}px", options.width)
        },
        options.error
    );
    Ok(())
}

fn parse_color(input: &str) -> Result<[u8; 4]> {
    let hex = input.strip_prefix('#').unwrap_or(input);
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("color must be a 6- or 8-digit hex value: {input}");
    }
    let mut rgba = [0u8, 0, 0, 255];
    for (index, component) in rgba.iter_mut().enumerate().take(hex.len() / 2) {
        *component = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)?;
    }
    Ok(rgba)
}

fn png_qr(
    code: &QrCode,
    margin: u32,
    width: u32,
    dark: [u8; 4],
    light: [u8; 4],
) -> Result<Vec<u8>> {
    let modules = code.width() as u32;
    let total = modules
        .checked_add(margin.saturating_mul(2))
        .ok_or_else(|| anyhow!("QR dimensions overflowed"))?;
    let pixel_count = width
        .checked_mul(width)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| anyhow!("PNG dimensions are too large"))?;
    let mut pixels = vec![0u8; pixel_count as usize];
    let colors = code.to_colors();
    for y in 0..width {
        let module_y = (y as u64 * total as u64 / width as u64) as u32;
        for x in 0..width {
            let module_x = (x as u64 * total as u64 / width as u64) as u32;
            let is_dark = module_x >= margin
                && module_y >= margin
                && module_x < margin + modules
                && module_y < margin + modules
                && colors[((module_y - margin) * modules + (module_x - margin)) as usize]
                    == Color::Dark;
            let color = if is_dark { dark } else { light };
            let offset = ((y * width + x) * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&color);
        }
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(
        &pixels,
        width,
        width,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png)
}

fn css_color(input: &str) -> String {
    let hex = input.strip_prefix('#').unwrap_or(input);
    format!("#{}", hex.to_ascii_lowercase())
}

fn svg_qr(code: &QrCode, margin: u32, dark: &str, light: &str) -> String {
    let modules = code.width() as u32;
    let total = modules + margin * 2;
    let colors = code.to_colors();
    let mut path = String::new();
    for y in 0..modules {
        for x in 0..modules {
            if colors[(y * modules + x) as usize] == Color::Dark {
                path.push_str(&format!("M{} {}h1v1h-1z", x + margin, y + margin));
            }
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {total} {total}\" shape-rendering=\"crispEdges\"><path fill=\"{}\" d=\"M0 0h{total}v{total}H0z\"/><path fill=\"{}\" d=\"{path}\"/></svg>\n",
        css_color(light),
        css_color(dark),
    )
}

fn terminal_qr(code: &QrCode, margin: u32) -> String {
    let modules = code.width() as i32;
    let margin = margin as i32;
    let total = modules + margin * 2;
    let colors = code.to_colors();
    let dark = |x: i32, y: i32| -> bool {
        if x < margin || y < margin || x >= margin + modules || y >= margin + modules {
            return false;
        }
        colors[((y - margin) * modules + (x - margin)) as usize] == Color::Dark
    };
    let mut output = String::new();
    let mut y = 0;
    while y < total {
        for x in 0..total {
            output.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        output.push('\n');
        y += 2;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_rgba_colors() {
        assert_eq!(parse_color("#112233").unwrap(), [0x11, 0x22, 0x33, 0xff]);
        assert_eq!(parse_color("11223344").unwrap(), [0x11, 0x22, 0x33, 0x44]);
        assert!(parse_color("red").is_err());
    }

    #[test]
    fn accepts_options_after_text() {
        let cli = Cli::try_parse_from([
            "justqr",
            "hello",
            "world",
            "--output",
            "custom.png",
            "--width",
            "256",
        ])
        .unwrap();

        assert_eq!(cli.text, ["hello", "world"]);
        assert_eq!(cli.output, Some(PathBuf::from("custom.png")));
        assert_eq!(cli.width, 256);
    }

    #[test]
    fn svg_has_quiet_zone_and_namespace() {
        let code = QrCode::new(b"hello").unwrap();
        let svg = svg_qr(&code, 4, "#000000", "#ffffff");
        assert!(svg.contains("xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains(&format!(
            "viewBox=\"0 0 {} {}\"",
            code.width() + 8,
            code.width() + 8
        )));
    }

    #[test]
    fn png_is_exact_requested_size() {
        let code = QrCode::new(b"hello").unwrap();
        let png = png_qr(&code, 4, 128, [0, 0, 0, 255], [255; 4]).unwrap();
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
        assert_eq!((image.width(), image.height()), (128, 128));
    }
}
