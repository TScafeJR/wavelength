//! The command line: render a page to a PNG, or compare pages against Chrome.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use wavelength::{Error, Page, Result, Viewport, report};

const USAGE: &str = "\
A from-scratch browser engine, measured against Chrome.

Usage:
  wavelength render <page.html> [--out FILE] [--width N] [--height N]
  wavelength compare <page.html>... [--out DIR] [--width N] [--height N] [--max-diff PERCENT]

Commands:
  render   Paint one page with the engine and write a PNG (default: render.png)
  compare  Render each page with the engine and with Chrome, then write the
           images, diff overlays and an index.html report (default: report/)

Options:
  --out PATH          Where to write the output
  --width N           Viewport width in pixels (default: 800)
  --height N          Viewport height in pixels (default: 600)
  --max-diff PERCENT  Exit with an error if any page differs by more than this
  -h, --help          Show this help
  -V, --version       Show the version

Environment:
  WAVELENGTH_CHROME  Path to the Chrome or Chromium binary used for references";

struct Options {
    pages: Vec<PathBuf>,
    out: Option<PathBuf>,
    viewport: Viewport,
    max_diff: Option<f64>,
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            if matches!(error, Error::Usage(_)) {
                eprintln!("Run with --help for usage.");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<String>) -> Result<()> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next().unwrap_or_default();
    match command.as_str() {
        "render" => render(parse_options(arguments)?),
        "compare" => compare(parse_options(arguments)?),
        "" | "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("wavelength {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(Error::Usage(format!("unknown command: {other}"))),
    }
}

fn parse_options(mut arguments: impl Iterator<Item = String>) -> Result<Options> {
    let mut options = Options {
        pages: Vec::new(),
        out: None,
        viewport: Viewport::default(),
        max_diff: None,
    };
    while let Some(argument) = arguments.next() {
        let mut value = |flag: &str| {
            arguments
                .next()
                .ok_or_else(|| Error::Usage(format!("{flag} needs a value")))
        };
        match argument.as_str() {
            "--out" | "-o" => options.out = Some(PathBuf::from(value("--out")?)),
            "--width" => options.viewport.width = dimension("--width", &value("--width")?)?,
            "--height" => options.viewport.height = dimension("--height", &value("--height")?)?,
            "--max-diff" => {
                let text = value("--max-diff")?;
                let percent = text
                    .parse::<f64>()
                    .ok()
                    .filter(|p| (0.0..=100.0).contains(p));
                options.max_diff = Some(percent.ok_or_else(|| {
                    Error::Usage(format!("--max-diff must be between 0 and 100, got {text}"))
                })?);
            }
            flag if flag.starts_with('-') => {
                return Err(Error::Usage(format!("unknown option: {flag}")));
            }
            _ => options.pages.push(PathBuf::from(argument)),
        }
    }
    Ok(options)
}

fn dimension(flag: &str, text: &str) -> Result<u32> {
    text.parse()
        .ok()
        .filter(|n| (1..=16_384).contains(n))
        .ok_or_else(|| Error::Usage(format!("{flag} must be between 1 and 16384, got {text}")))
}

fn render(options: Options) -> Result<()> {
    let [page] = &options.pages[..] else {
        return Err(Error::Usage("render takes exactly one page".to_owned()));
    };
    let out = options.out.unwrap_or_else(|| PathBuf::from("render.png"));
    let html = fs::read_to_string(page)?;
    Page::load(&html, options.viewport).render().save(&out)?;
    println!("wrote {}", out.display());
    Ok(())
}

fn compare(options: Options) -> Result<()> {
    if options.pages.is_empty() {
        return Err(Error::Usage("compare needs at least one page".to_owned()));
    }
    let out = options.out.unwrap_or_else(|| PathBuf::from("report"));
    let entries = report::run(&options.pages, &out, options.viewport)?;
    for entry in &entries {
        println!(
            "{:<24} {:>9} of {} pixels differ ({:.3}%)",
            entry.name, entry.differing, entry.total, entry.percent
        );
    }
    println!("report: {}", out.join("index.html").display());

    if let Some(limit) = options.max_diff {
        let over: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.percent > limit)
            .map(|entry| entry.name.as_str())
            .collect();
        if !over.is_empty() {
            return Err(Error::Parity(format!(
                "{} differ from Chrome by more than {limit}%",
                over.join(", ")
            )));
        }
    }
    Ok(())
}
