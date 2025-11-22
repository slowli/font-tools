//! CLI integration tests.

// #![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

use term_transcript::{
    svg::{LineNumbers, ScrollOptions, Template, TemplateOptions},
    test::{MatchKind, TestConfig},
    ShellOptions,
};

const EXE_PATH: &str = env!("CARGO_BIN_EXE_font-subset");
const FONT_PATH: &str = "tests/RobotoMono.ttf";

#[derive(Debug)]
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Default for Sandbox {
    fn default() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        fs::copy(FONT_PATH, dir.path().join("RobotoMono.ttf")).expect("cannot copy test font");

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

fn scroll_template() -> Template {
    let template_options = TemplateOptions {
        window_frame: true,
        scroll: Some(ScrollOptions::default()),
        line_numbers: Some(LineNumbers::ContinuousOutputs),
        ..TemplateOptions::default()
    };
    Template::pure_svg(template_options)
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
        .with_template(scroll_template())
        .test(snapshot("info"), ["font-subset info RobotoMono.ttf"]);
}

#[test]
fn subsetting_basics() {
    let sandbox = Sandbox::default();
    test_config(Some(sandbox.dir.path()))
        .with_template(scroll_template())
        .test(
            snapshot("subset"),
            [
                "font-subset subset --str \"Hello world!\" -o subset.woff RobotoMono.ttf",
                "font-subset info subset.woff",
            ],
        );
}
