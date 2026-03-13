use crate::engine::nav_provider::StokesNavigationProvider;
use crate::engine::{Engine, EngineConfig};
use crate::networking;
use crate::shell_provider::StokesShellProvider;
use blitz_traits::shell::Viewport;
use serde::Serialize;
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::unbounded_channel;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

const REPORT_NODE_ID: &str = "stokes-wpt-results";
const DEFAULT_MANIFEST: &str = "wpt/manifests/smoke.txt";
const DEFAULT_EXPECTATIONS: &str = "wpt/expectations/known-failures.txt";
const DEFAULT_OUTPUT: &str = "wpt/results/latest.json";
const DEFAULT_WPT_ROOT: &str = "third_party/wpt";
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8000";

#[derive(Debug)]
struct WptOptions {
    manifest: PathBuf,
    expectations: PathBuf,
    output: PathBuf,
    wpt_root: PathBuf,
    base_url: String,
    timeout_ms: u64,
    poll_ms: u64,
    max_tests: Option<usize>,
    filter: Option<String>,
    single_test: Option<String>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
struct RunOutput {
    started_unix_ms: u128,
    duration_ms: u128,
    config: OutputConfig,
    summary: RunSummary,
    results: Vec<TestResult>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
struct OutputConfig {
    manifest: String,
    expectations: String,
    output: String,
    wpt_root: String,
    base_url: String,
    timeout_ms: u64,
    poll_ms: u64,
    max_tests: Option<usize>,
    filter: Option<String>,
}

#[derive(Debug, Serialize, serde::Deserialize, Default)]
struct RunSummary {
    total: usize,
    passed: usize,
    failed: usize,
    expected_failures: usize,
    regressions: usize,
    unexpected_passes: usize,
    skipped: usize,
    harness_timeouts: usize,
}

#[derive(Debug, Serialize, serde::Deserialize)]
struct TestResult {
    path: String,
    url: String,
    outcome: Outcome,
    harness_status: Option<i32>,
    harness_message: Option<String>,
    failing_subtests: Vec<SubtestFailure>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Pass,
    Fail,
    ExpectedFail,
    Regression,
    UnexpectedPass,
    Skipped,
}

#[derive(Debug, Serialize, serde::Deserialize)]
struct SubtestFailure {
    name: String,
    status: i32,
    message: String,
}

#[derive(Debug, serde::Deserialize)]
struct HarnessReport {
    status: HarnessStatus,
    tests: Vec<HarnessSubtest>,
}

#[derive(Debug, serde::Deserialize)]
struct HarnessStatus {
    status: i32,
    message: String,
}

#[derive(Debug, serde::Deserialize)]
struct HarnessSubtest {
    name: String,
    status: i32,
    message: String,
}

pub(crate) async fn run_from_args(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = WptOptions::parse(args)?;
    let expected_failures = load_expectations(&options.expectations)?;

    let mut tests = if let Some(single_test) = options.single_test.as_ref() {
        vec![normalize_manifest_path(single_test)]
    } else {
        let manifest_entries = load_manifest(&options.manifest)?;
        let mut selected = select_tests(manifest_entries, &options)?;
        if selected.is_empty() {
            return Err(format!(
                "No tests selected. Check manifest path '{}' and filter settings.",
                options.manifest.display()
            )
            .into());
        }

        if let Some(limit) = options.max_tests {
            selected.truncate(limit);
        }
        selected
    };

    println!("Running {} WPT tests against {}", tests.len(), options.base_url);

    let started = Instant::now();
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut engine = build_engine();
    let mut results = Vec::with_capacity(tests.len());
    let current_exe = if options.single_test.is_none() {
        Some(std::env::current_exe()?)
    } else {
        None
    };

    for (index, path) in tests.iter().enumerate() {
        println!("[{}/{}] {}", index + 1, tests.len(), path);
        let result = if let Some(exe) = current_exe.as_ref() {
            run_one_test_isolated(exe, path, &options, &expected_failures).await
        } else {
            run_one_test(&mut engine, path, &options, &expected_failures).await
        };
        print_result_line(&result);
        results.push(result);
    }

    let summary = summarize(&results);
    let output = RunOutput {
        started_unix_ms,
        duration_ms: started.elapsed().as_millis(),
        config: OutputConfig {
            manifest: options.manifest.display().to_string(),
            expectations: options.expectations.display().to_string(),
            output: options.output.display().to_string(),
            wpt_root: options.wpt_root.display().to_string(),
            base_url: options.base_url.clone(),
            timeout_ms: options.timeout_ms,
            poll_ms: options.poll_ms,
            max_tests: options.max_tests,
            filter: options.filter.clone(),
        },
        summary,
        results,
    };

    if let Some(parent) = options.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&options.output, serde_json::to_vec_pretty(&output)?)?;

    println!("\nWPT run complete. Results written to {}", options.output.display());
    println!(
        "pass={} fail={} expected_fail={} regressions={} unexpected_pass={} skipped={}",
        output.summary.passed,
        output.summary.failed,
        output.summary.expected_failures,
        output.summary.regressions,
        output.summary.unexpected_passes,
        output.summary.skipped
    );

    Ok(())
}

impl WptOptions {
    fn parse(args: &[String]) -> Result<Self, Box<dyn Error>> {
        let mut options = WptOptions {
            manifest: PathBuf::from(DEFAULT_MANIFEST),
            expectations: PathBuf::from(DEFAULT_EXPECTATIONS),
            output: PathBuf::from(DEFAULT_OUTPUT),
            wpt_root: PathBuf::from(DEFAULT_WPT_ROOT),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_ms: 8_000,
            poll_ms: 25,
            max_tests: None,
            filter: None,
            single_test: None,
        };

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--manifest" => {
                    i += 1;
                    options.manifest = PathBuf::from(arg_value(args, i, "--manifest")?);
                }
                "--expectations" => {
                    i += 1;
                    options.expectations = PathBuf::from(arg_value(args, i, "--expectations")?);
                }
                "--output" => {
                    i += 1;
                    options.output = PathBuf::from(arg_value(args, i, "--output")?);
                }
                "--wpt-root" => {
                    i += 1;
                    options.wpt_root = PathBuf::from(arg_value(args, i, "--wpt-root")?);
                }
                "--base-url" => {
                    i += 1;
                    options.base_url = arg_value(args, i, "--base-url")?.to_string();
                }
                "--timeout-ms" => {
                    i += 1;
                    options.timeout_ms = arg_value(args, i, "--timeout-ms")?.parse()?;
                }
                "--poll-ms" => {
                    i += 1;
                    options.poll_ms = arg_value(args, i, "--poll-ms")?.parse()?;
                }
                "--max-tests" => {
                    i += 1;
                    options.max_tests = Some(arg_value(args, i, "--max-tests")?.parse()?);
                }
                "--filter" => {
                    i += 1;
                    options.filter = Some(arg_value(args, i, "--filter")?.to_string());
                }
                "--single-test" => {
                    i += 1;
                    options.single_test = Some(arg_value(args, i, "--single-test")?.to_string());
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                unknown => {
                    return Err(format!("Unknown argument '{}'. Use --help for usage.", unknown).into())
                }
            }
            i += 1;
        }

        Ok(options)
    }
}

fn arg_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, Box<dyn Error>> {
    args.get(index)
        .map(|s| s.as_str())
        .ok_or_else(|| format!("Missing value for {}", flag).into())
}

fn print_help() {
    println!(
        "Usage: stokes-browser --wpt-run [options]\n\
         \n\
         Options:\n\
         --manifest <path>       Manifest of WPT tests to run (default: {DEFAULT_MANIFEST})\n\
         --expectations <path>   Expected failures list (default: {DEFAULT_EXPECTATIONS})\n\
         --output <path>         JSON output file (default: {DEFAULT_OUTPUT})\n\
         --wpt-root <path>       Local WPT checkout path (default: {DEFAULT_WPT_ROOT})\n\
         --base-url <url>        URL where WPT server is running (default: {DEFAULT_BASE_URL})\n\
         --timeout-ms <n>        Per-test timeout in milliseconds (default: 8000)\n\
         --poll-ms <n>           Poll interval while waiting for completion (default: 25)\n\
         --max-tests <n>         Cap number of tests from manifest\n\
         --filter <text>         Run only tests whose path contains the text\n\
         --single-test <path>    Internal: run exactly one test path in-process"
    );
}

async fn run_one_test_isolated(
    exe: &Path,
    path: &str,
    options: &WptOptions,
    expected_failures: &HashSet<String>,
) -> TestResult {
    let expected_fail = expected_failures.contains(path);
    let url = format!("{}/{}", options.base_url.trim_end_matches('/'), path);
    let temp_output = temp_output_path(path);

    let mut cmd = Command::new(exe);
    cmd.arg("--wpt-run")
        .arg("--single-test")
        .arg(path)
        .arg("--expectations")
        .arg(&options.expectations)
        .arg("--output")
        .arg(&temp_output)
        .arg("--wpt-root")
        .arg(&options.wpt_root)
        .arg("--base-url")
        .arg(&options.base_url)
        .arg("--timeout-ms")
        .arg(options.timeout_ms.to_string())
        .arg("--poll-ms")
        .arg(options.poll_ms.to_string());

    let output = match cmd.output().await {
        Ok(output) => output,
        Err(err) => {
            return crash_result(
                path,
                url,
                expected_fail,
                format!("failed to launch isolated test runner: {}", err),
            )
        }
    };

    if output.status.success() {
        let result = match read_single_result(&temp_output) {
            Ok(result) => result,
            Err(err) => crash_result(
                path,
                url,
                expected_fail,
                format!(
                    "isolated runner produced unreadable result: {}{}",
                    err,
                    stderr_suffix(&output)
                ),
            ),
        };
        let _ = fs::remove_file(&temp_output);
        return result;
    }

    let signal = exit_signal(output.status);
    let error = if let Some(sig) = signal {
        format!(
            "isolated runner crashed with {} (signal {}){}",
            signal_name(sig),
            sig,
            stderr_suffix(&output)
        )
    } else {
        format!(
            "isolated runner exited with code {:?}{}",
            output.status.code(),
            stderr_suffix(&output)
        )
    };

    let _ = fs::remove_file(&temp_output);
    crash_result(path, url, expected_fail, error)
}

fn temp_output_path(path: &str) -> PathBuf {
    let mut sanitized = path.replace(['/', '\\'], "_");
    sanitized = sanitized
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' { ch } else { '_' })
        .collect();

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    std::env::temp_dir().join(format!(
        "stokes-wpt-{}-{}-{}.json",
        std::process::id(),
        unique,
        sanitized
    ))
}

fn read_single_result(path: &Path) -> Result<TestResult, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let output: RunOutput = serde_json::from_slice(&bytes)?;
    output
        .results
        .into_iter()
        .next()
        .ok_or_else(|| "isolated runner output did not include any test results".into())
}

fn crash_result(path: &str, url: String, expected_fail: bool, error: String) -> TestResult {
    TestResult {
        path: path.to_string(),
        url,
        outcome: if expected_fail {
            Outcome::ExpectedFail
        } else {
            Outcome::Regression
        },
        harness_status: None,
        harness_message: None,
        failing_subtests: Vec::new(),
        error: Some(error),
    }
}

fn stderr_suffix(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        String::new()
    } else {
        format!("; stderr: {}", stderr)
    }
}

fn exit_signal(status: ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        status.signal()
    }

    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

fn signal_name(sig: i32) -> &'static str {
    match sig {
        11 => "SIGSEGV",
        6 => "SIGABRT",
        4 => "SIGILL",
        8 => "SIGFPE",
        _ => "signal",
    }
}

fn build_engine() -> Engine {
    let (shell_tx, _shell_rx) = unbounded_channel();
    let shell_provider = Arc::new(StokesShellProvider::new(shell_tx));

    let (nav_tx, _nav_rx) = unbounded_channel();
    let navigation_provider = Arc::new(StokesNavigationProvider::new(nav_tx));

    let viewport = Viewport {
        color_scheme: Default::default(),
        window_size: (1280, 720),
        hidpi_scale: 1.0,
        zoom: 1.0,
    };

    let mut config = EngineConfig::default();
    config.debug_js = false;
    config.debug_net = false;

    Engine::new(config, viewport, shell_provider, navigation_provider)
}

fn load_manifest(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let manifest_text = fs::read_to_string(path)?;
    let entries = manifest_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(normalize_manifest_path)
        .collect();
    Ok(entries)
}

fn load_expectations(path: &Path) -> Result<HashSet<String>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }

    let text = fs::read_to_string(path)?;
    let entries = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(normalize_manifest_path)
        .collect();
    Ok(entries)
}

fn select_tests(entries: Vec<String>, options: &WptOptions) -> Result<Vec<String>, Box<dyn Error>> {
    let mut selected = Vec::new();

    for path in entries {
        let absolute = options.wpt_root.join(&path);
        if absolute.exists() {
            selected.push(path);
        } else {
            println!("Skipping missing manifest entry: {}", path);
        }
    }

    if let Some(filter) = options.filter.as_ref() {
        selected.retain(|path| path.contains(filter));
    }

    Ok(selected)
}

fn normalize_manifest_path(path: &str) -> String {
    path.trim_start_matches('/')
        .replace('\\', "/")
}

async fn run_one_test(
    engine: &mut Engine,
    path: &str,
    options: &WptOptions,
    expected_failures: &HashSet<String>,
) -> TestResult {
    let url = format!("{}/{}", options.base_url.trim_end_matches('/'), path);
    let expected_fail = expected_failures.contains(path);

    let source = match networking::fetch(&url, &engine.config.user_agent) {
        Ok(contents) => contents,
        Err(err) => {
            return TestResult {
                path: path.to_string(),
                url,
                outcome: if expected_fail {
                    Outcome::ExpectedFail
                } else {
                    Outcome::Regression
                },
                harness_status: None,
                harness_message: None,
                failing_subtests: Vec::new(),
                error: Some(format!("fetch failed: {}", err)),
            }
        }
    };

    if !source.contains("testharness.js") {
        return TestResult {
            path: path.to_string(),
            url,
            outcome: Outcome::Skipped,
            harness_status: None,
            harness_message: None,
            failing_subtests: Vec::new(),
            error: Some("non-testharness test (currently unsupported)".to_string()),
        };
    }

    let test_html = inject_reporter(&source);
    if let Err(err) = engine.navigate(&url, test_html, true, false).await {
        return TestResult {
            path: path.to_string(),
            url,
            outcome: if expected_fail {
                Outcome::ExpectedFail
            } else {
                Outcome::Regression
            },
            harness_status: None,
            harness_message: None,
            failing_subtests: Vec::new(),
            error: Some(format!("navigate failed: {}", err)),
        };
    }

    let started = Instant::now();
    let timeout = Duration::from_millis(options.timeout_ms);
    let poll = Duration::from_millis(options.poll_ms);

    while started.elapsed() <= timeout {
        let now = started.elapsed().as_secs_f64();
        engine.resolve(now);

        if let Some(raw_json) = extract_report_json(engine) {
            return classify_result(path, url, raw_json, expected_fail);
        }

        tokio::time::sleep(poll).await;
    }

    TestResult {
        path: path.to_string(),
        url,
        outcome: if expected_fail {
            Outcome::ExpectedFail
        } else {
            Outcome::Regression
        },
        harness_status: Some(2),
        harness_message: Some("timeout waiting for testharness completion".to_string()),
        failing_subtests: Vec::new(),
        error: Some(format!("timed out after {} ms", options.timeout_ms)),
    }
}

fn inject_reporter(html: &str) -> String {
    const REPORTER: &str = r#"<script>
(function() {
  function finish(payload) {
    var node = document.getElementById('stokes-wpt-results');
    if (!node) {
      node = document.createElement('script');
      node.type = 'application/json';
      node.id = 'stokes-wpt-results';
      (document.body || document.documentElement).appendChild(node);
    }
    node.textContent = JSON.stringify(payload);
  }

  function register() {
    if (typeof add_completion_callback === 'function') {
      add_completion_callback(function(tests, status) {
        finish({
          status: {
            status: status && typeof status.status === 'number' ? status.status : 1,
            message: status && status.message ? String(status.message) : ''
          },
          tests: (tests || []).map(function(test) {
            return {
              name: String(test.name || ''),
              status: typeof test.status === 'number' ? test.status : 1,
              message: test.message ? String(test.message) : ''
            };
          })
        });
      });
      return;
    }
    setTimeout(register, 0);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', register, { once: true });
  } else {
    register();
  }
})();
</script>"#;

    if let Some(index) = html.rfind("</body>") {
        let mut out = String::with_capacity(html.len() + REPORTER.len());
        out.push_str(&html[..index]);
        out.push_str(REPORTER);
        out.push_str(&html[index..]);
        return out;
    }

    let mut out = String::with_capacity(html.len() + REPORTER.len());
    out.push_str(html);
    out.push_str(REPORTER);
    out
}

fn extract_report_json(engine: &Engine) -> Option<String> {
    let dom = engine.dom();
    let node = dom.query_selector(&format!("#{REPORT_NODE_ID}")).into_iter().next()?;
    let text = node.text_content();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn classify_result(path: &str, url: String, raw_json: String, expected_fail: bool) -> TestResult {
    let parsed: HarnessReport = match serde_json::from_str(&raw_json) {
        Ok(report) => report,
        Err(err) => {
            return TestResult {
                path: path.to_string(),
                url,
                outcome: if expected_fail {
                    Outcome::ExpectedFail
                } else {
                    Outcome::Regression
                },
                harness_status: None,
                harness_message: None,
                failing_subtests: Vec::new(),
                error: Some(format!("invalid reporter json: {}", err)),
            }
        }
    };

    let failing_subtests: Vec<SubtestFailure> = parsed
        .tests
        .iter()
        .filter(|test| test.status != 0)
        .map(|test| SubtestFailure {
            name: test.name.clone(),
            status: test.status,
            message: test.message.clone(),
        })
        .collect();

    let raw_pass = parsed.status.status == 0 && failing_subtests.is_empty();
    let outcome = match (raw_pass, expected_fail) {
        (true, true) => Outcome::UnexpectedPass,
        (true, false) => Outcome::Pass,
        (false, true) => Outcome::ExpectedFail,
        (false, false) => Outcome::Regression,
    };

    TestResult {
        path: path.to_string(),
        url,
        outcome,
        harness_status: Some(parsed.status.status),
        harness_message: if parsed.status.message.is_empty() {
            None
        } else {
            Some(parsed.status.message)
        },
        failing_subtests,
        error: None,
    }
}

fn summarize(results: &[TestResult]) -> RunSummary {
    let mut summary = RunSummary::default();

    for result in results {
        summary.total += 1;
        match result.outcome {
            Outcome::Pass => summary.passed += 1,
            Outcome::Fail => summary.failed += 1,
            Outcome::ExpectedFail => {
                summary.failed += 1;
                summary.expected_failures += 1;
            }
            Outcome::Regression => {
                summary.failed += 1;
                summary.regressions += 1;
            }
            Outcome::UnexpectedPass => {
                summary.passed += 1;
                summary.unexpected_passes += 1;
            }
            Outcome::Skipped => summary.skipped += 1,
        }

        if result
            .harness_message
            .as_deref()
            .unwrap_or_default()
            .contains("timeout")
        {
            summary.harness_timeouts += 1;
        }
    }

    summary
}

fn print_result_line(result: &TestResult) {
    let label = match result.outcome {
        Outcome::Pass => "PASS",
        Outcome::Fail => "FAIL",
        Outcome::ExpectedFail => "XFAIL",
        Outcome::Regression => "REGRESSION",
        Outcome::UnexpectedPass => "XPASS",
        Outcome::Skipped => "SKIP",
    };

    if let Some(err) = &result.error {
        println!("  {} - {} ({})", label, result.path, err);
    } else if result.failing_subtests.is_empty() {
        println!("  {} - {}", label, result.path);
    } else {
        println!(
            "  {} - {} ({} failing subtests)",
            label,
            result.path,
            result.failing_subtests.len()
        );
    }
}

