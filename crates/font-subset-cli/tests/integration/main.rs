//! CLI integration tests.

use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

use term_transcript::{
    svg::{self, Template, TemplateOptions},
    test::{MatchKind, TestConfig},
    ShellOptions,
};

use crate::subsetter::SimpleSubsetter;

mod subsetter;

const EXE_PATH: &str = env!("CARGO_BIN_EXE_font-subset");
const ROBOTO_MONO_PATH: &str = "tests/RobotoMono.ttf";
const FIRA_MONO_PATH: &str = "tests/FiraMono.ttf";

#[derive(Debug)]
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Default for Sandbox {
    fn default() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        fs::copy(ROBOTO_MONO_PATH, dir.path().join("RobotoMono.ttf"))
            .expect("cannot copy test font");
        fs::copy(FIRA_MONO_PATH, dir.path().join("FiraMono.ttf")).expect("cannot copy test font");

        Self { dir }
    }
}

fn assert_success(output: &Output) {
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "Program exited abnormally: {:?}\n--- stdout ---{stdout}\n--- stderr ---\n{stderr}",
            output.status
        );
    }
}

fn template(scroll: bool, title: &str) -> Template {
    let subsetter = SimpleSubsetter::roboto_mono().unwrap();
    let template_options = TemplateOptions {
        window: Some(svg::WindowOptions {
            title: title.to_owned(),
        }),
        width: NonZeroUsize::new(752).unwrap(),
        scroll: scroll.then(svg::ScrollOptions::default),
        line_numbers: Some(svg::LineNumberingOptions {
            scope: svg::LineNumbers::ContinuousOutputs,
            ..svg::LineNumberingOptions::default()
        }),
        ..TemplateOptions::default().with_font_embedder(subsetter)
    };
    Template::pure_svg(template_options.validated().unwrap())
}

fn test_config(current_dir: Option<&Path>) -> TestConfig {
    let mut shell_options = ShellOptions::default()
        .with_env("CLICOLOR_FORCE", "1")
        .with_cargo_path()
        .with_io_timeout(Duration::from_secs(1));
    if let Some(dir) = current_dir {
        shell_options = shell_options.with_current_dir(dir);
    }
    TestConfig::new(shell_options).with_match_kind(MatchKind::Precise)
}

fn snapshot(name: &str) -> PathBuf {
    let mut snapshot_path = Path::new("examples").join(name);
    snapshot_path.set_extension("svg");
    snapshot_path
}

#[test]
fn help_subcommand_works() {
    let output = Command::new(EXE_PATH).arg("help").output().unwrap();
    assert_success(&output);
}

#[test]
fn printing_font_info() {
    let sandbox = Sandbox::default();
    test_config(Some(sandbox.dir.path()))
        .with_template(template(true, "Printing font info"))
        .test(
            snapshot("info"),
            ["font-subset info --verbose RobotoMono.ttf"],
        );
}

#[test]
fn subsetting_basics() {
    let sandbox = Sandbox::default();
    test_config(Some(sandbox.dir.path()))
        .with_template(template(false, "Font subsetting"))
        .test(
            snapshot("subset"),
            [
                "font-subset subset --str \"Hello world!\" -o subset.woff FiraMono.ttf",
                "font-subset info subset.woff",
            ],
        );
}

#[test]
fn subsetting_dropping_vars() {
    let sandbox = Sandbox::default();
    test_config(Some(sandbox.dir.path()))
        .with_template(template(true, "Subsetting + dropping var axes"))
        .test(
            snapshot("subset-drop-var"),
            [
                "font-subset subset --chars \"a-z0-9\" --drop-var -o plain.ttf RobotoMono.ttf",
                "font-subset info --verbose plain.ttf",
            ],
        );
}

#[cfg(unix)] // `cmd` doesn't support streaming (of course)
#[test]
fn streaming() {
    let sandbox = Sandbox::default();
    test_config(Some(sandbox.dir.path()))
        .with_template(template(false, "Streaming API"))
        .test(
            snapshot("subset-streaming"),
            ["font-subset subset --ascii -o - --format woff2 - < FiraMono.ttf \\\n  | font-subset info -"],
        );
}
