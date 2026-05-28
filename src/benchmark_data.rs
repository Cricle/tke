pub(crate) fn repeated_benchmark_code(lines: usize) -> String {
    (0..lines)
        .map(|idx| match idx % 6 {
            0 => format!("pub struct Struct{idx} {{"),
            1 => format!("    field_{idx}: usize,"),
            2 => "}".to_owned(),
            3 => format!("pub fn function_{idx}() {{"),
            4 => format!("    println!(\"{{}}\", {idx});"),
            _ => "}".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_benchmark_numbered_code(lines: usize) -> String {
    repeated_benchmark_code(lines)
        .lines()
        .enumerate()
        .map(|(idx, line)| format!("{:>6}\t{line}", idx + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_benchmark_search() -> String {
    (0..140)
        .map(|idx| format!("src/lib.rs:{}:pub fn alpha_{}() {{}}", idx + 1, idx))
        .chain((0..80).map(|idx| format!("src/main.rs:{}:pub struct Beta{};", idx + 1, idx)))
        .chain((0..40).map(|idx| format!("tests/lib.rs:{}:impl Gamma{} {{}}", idx + 1, idx)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_benchmark_paths(count: usize) -> String {
    (0..count)
        .map(|idx| format!("/root/project/target/debug/incremental/tke/build-artifact-{idx:04}.o"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_candidate_root_cause_paths(count: usize) -> String {
    let mut rows = vec![
        "/root/project/src/tests.rs".to_owned(),
        "/root/project/src/e2e_report.rs".to_owned(),
        "/root/project/src/app.rs".to_owned(),
        "/root/project/src/benchmark.rs".to_owned(),
    ];
    while rows.len() < count {
        rows.push(format!(
            "/root/project/src/candidate_module_{:03}.rs",
            rows.len()
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_ls_long() -> String {
    let mut rows = vec!["total 160".to_owned()];
    for idx in 0..40 {
        rows.push(format!(
            "-rw-r--r-- 1 root root {:>5} May 23 17:{:02} module_{idx:02}.rs",
            1024 + idx * 13,
            idx % 60
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_ls_names(count: usize) -> String {
    (0..count)
        .map(|idx| {
            if idx % 5 == 0 {
                format!("module_{idx:03}")
            } else {
                format!("module_{idx:03}.rs")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_benchmark_wc() -> String {
    [
        "  3816 115848 src/lib.rs",
        "    82   3580 README.md",
        "  3898 119428 total",
    ]
    .join("\n")
}

pub(crate) fn repeated_benchmark_pretty_json() -> String {
    let items = (0..160)
        .map(|idx| {
            format!(
                "    {{\n      \"id\": {idx},\n      \"name\": \"item-{idx:03}\",\n      \"ok\": true,\n      \"tags\": [\"alpha\", \"beta\", \"gamma\"],\n      \"meta\": {{ \"owner\": \"team-{idx:02}\", \"retries\": {} }}\n    }}",
                idx % 5
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "{{\n  \"ok\": true,\n  \"source\": \"demo\",\n  \"items\": [\n{items}\n  ],\n  \"meta\": {{\n    \"count\": 160,\n    \"kind\": \"sample\",\n    \"generated_by\": \"benchmark\"\n  }}\n}}"
    )
}

pub(crate) fn repeated_benchmark_http_json_response() -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 4096\r\ndate: Fri, 22 May 2026 07:25:31 GMT\r\n\r\n{}",
        repeated_benchmark_pretty_json()
    )
}

pub(crate) fn repeated_benchmark_named_diff(path: &str, symbol_prefix: &str) -> String {
    let mut rows = vec![
        format!("diff --git a/{path} b/{path}"),
        "index 1234567..89abcde 100644".to_owned(),
        format!("--- a/{path}"),
        format!("+++ b/{path}"),
    ];
    for idx in 0..120 {
        rows.push(format!(
            "@@ -{},3 +{},6 @@ pub fn {}_{}() {{",
            idx * 10 + 1,
            idx * 10 + 1,
            symbol_prefix,
            idx
        ));
        rows.push("-    old_call();".to_owned());
        rows.push("+    new_call();".to_owned());
        rows.push(format!("+    extra_line_{}();", idx));
        rows.push("+    trace_call();".to_owned());
        rows.push(" }".to_owned());
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_multi_file_diff(files: &[(&str, &str)]) -> String {
    files
        .iter()
        .map(|(path, symbol_prefix)| repeated_benchmark_named_diff(path, symbol_prefix))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_benchmark_diff() -> String {
    repeated_benchmark_named_diff("src/lib.rs", "function")
}

pub(crate) fn repeated_benchmark_build_log(kind: &str) -> String {
    match kind {
        "cargo" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("Compiling crate_{idx:03} v0.1.0"));
            }
            rows.push("Running unittests src/lib.rs (target/debug/deps/demo)".to_owned());
            rows.push(
                "test result: FAILED. 117 passed; 3 failed; 0 ignored; 0 measured".to_owned(),
            );
            rows.push("warning: deprecated config key".to_owned());
            rows.push("error: test failed, to rerun pass --lib".to_owned());
            rows.join("\n")
        }
        "pytest" => {
            let mut rows = repeated_lines(".", 120)
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            rows.push(
                "FAILED tests/test_parser.py::test_invalid_input - AssertionError".to_owned(),
            );
            rows.push("2 passed, 1 failed, 1 skipped in 3.12s".to_owned());
            rows.push("warning: deprecated fixture used".to_owned());
            rows.join("\n")
        }
        "python-unittest" => {
            let mut rows = Vec::new();
            for idx in 0..117 {
                rows.push(format!(
                    "test_case_{idx:03} (tests.test_suite.CaseSuite.test_case_{idx:03}) ... ok"
                ));
            }
            rows.push("FAILED (failures=2, errors=1, skipped=3)".to_owned());
            rows.push("Ran 123 tests in 3.12s".to_owned());
            rows.push("Traceback (most recent call last):".to_owned());
            rows.push("AssertionError: expected parser state".to_owned());
            rows.join("\n")
        }
        "npm" | "pnpm" | "yarn" | "bun" | "node" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("PASS src/suite_{idx:03}.test.ts"));
            }
            rows.push("FAIL src/parser.test.ts".to_owned());
            rows.push("Tests: 120 passed, 1 failed, 2 skipped, 123 total".to_owned());
            rows.push("error: command exited with code 1".to_owned());
            rows.join("\n")
        }
        "dotnet" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("Passed TestCase.Group{idx:03}"));
            }
            rows.push(
                "Failed!  - Failed:     3, Passed:   117, Skipped:     0, Total:   120".to_owned(),
            );
            rows.push("error CS1002: ; expected".to_owned());
            rows.join("\n")
        }
        "go" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("ok   github.com/acme/mod/pkg{idx:03} 0.013s"));
            }
            rows.push("--- FAIL: TestParser (0.00s)".to_owned());
            rows.push("FAIL".to_owned());
            rows.push("panic: runtime error: index out of range".to_owned());
            rows.join("\n")
        }
        "cmake" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!(
                    "[{}/200] Building CXX object src/module_{idx:03}.o",
                    idx + 1
                ));
            }
            rows.push(
                "CMake Error at src/CMakeLists.txt:17 (add_executable): target sources missing"
                    .to_owned(),
            );
            rows.push("FAILED: app".to_owned());
            rows.join("\n")
        }
        "ctest" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("Test #{}: case_{idx:03} ... Passed", idx + 1));
            }
            rows.push("99% tests passed, 1 tests failed out of 120".to_owned());
            rows.push("The following tests FAILED:".to_owned());
            rows.push(" 42 - parser_test (Failed)".to_owned());
            rows.join("\n")
        }
        "make" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("[{:>3}%] Built target module_{idx:03}", idx % 100));
            }
            rows.push("make: *** [Makefile:42: test] Error 2".to_owned());
            rows.push("warning: stale generated file".to_owned());
            rows.join("\n")
        }
        "ninja" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!(
                    "[{}/120] Building CXX object core_{idx:03}.o",
                    idx + 1
                ));
            }
            rows.push("ninja: build stopped: subcommand failed.".to_owned());
            rows.push("FAILED: build/tests/parser_test".to_owned());
            rows.join("\n")
        }
        "pip" | "uv" | "poetry" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("Collecting package_{idx:03}"));
            }
            rows.push("Successfully installed demo-1.0 helper-2.0 toolkit-3.1".to_owned());
            rows.push("warning: Retrying (Retry(total=4, connect=None))".to_owned());
            rows.join("\n")
        }
        "mvn" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("[INFO] Building parser-module-{idx:03} 1.0.0"));
            }
            rows.push("[INFO] BUILD FAILURE".to_owned());
            rows.push("[ERROR] Tests run: 120, Failures: 1, Errors: 0, Skipped: 0".to_owned());
            rows.join("\n")
        }
        "gradle" | "javac" | "java" | "bundle" | "composer" => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("> Task :module{idx:03}:compileJava"));
            }
            rows.push("BUILD FAILED in 12s".to_owned());
            rows.push("1 test completed, 1 failed".to_owned());
            rows.join("\n")
        }
        _ => {
            let mut rows = Vec::new();
            for idx in 0..120 {
                rows.push(format!("{kind}: step {idx:03} finished"));
            }
            rows.push(format!("{kind}: warning: deprecated config key"));
            rows.push(format!("{kind}: error: build failed at target 007"));
            rows.join("\n")
        }
    }
}

pub(crate) fn repeated_benchmark_ps() -> String {
    let mut rows = vec![
        "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND".to_owned(),
    ];
    for idx in 0..24 {
        rows.push(format!(
            "root        {:>4}  {:>3}.{:1}  1.2 357624 101588 pts/1   Sl+  08:08   0:00 /usr/bin/process-{} --flag value",
            3000 + idx,
            9 - (idx / 3),
            idx % 10,
            idx
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_ss() -> String {
    let mut rows = vec![
        "Netid State  Recv-Q Send-Q Local Address:Port  Peer Address:Port  Process".to_owned(),
    ];
    for idx in 0..20 {
        rows.push(format!(
            "tcp   LISTEN 0      4096   127.0.0.1:{}       0.0.0.0:*          users:((\"svc-{}\",pid={},fd=9))",
            8000 + idx,
            idx,
            4200 + idx
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_netstat() -> String {
    let mut rows = vec![
        "Proto Recv-Q Send-Q Local Address           Foreign Address         State       PID/Program name"
            .to_owned(),
    ];
    for idx in 0..20 {
        rows.push(format!(
            "tcp        0      0 127.0.0.1:{}          0.0.0.0:*               LISTEN      {}/service-{}",
            9000 + idx,
            5200 + idx,
            idx
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_systemctl() -> String {
    let mut rows =
        vec!["UNIT                         LOAD   ACTIVE SUB     DESCRIPTION".to_owned()];
    for idx in 0..40 {
        rows.push(format!(
            "service-{idx:02}.service      loaded active running Sample Service {idx:02}"
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_docker_ps() -> String {
    let mut rows = vec![
        "CONTAINER ID   IMAGE          COMMAND                  CREATED         STATUS         PORTS                    NAMES".to_owned(),
    ];
    for idx in 0..16 {
        rows.push(format!(
            "abcde{idx:07}   app:{idx:02}        \"/bin/server --flag\"   {} hours ago   Up {} hours   127.0.0.1:{}->8080/tcp   app-{}",
            idx + 1,
            idx + 1,
            7000 + idx,
            idx
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_du() -> String {
    (0..32)
        .map(|idx| format!("{:>4}M\t/root/project/module_{idx:02}", 8 + idx))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_benchmark_df() -> String {
    let mut rows = vec!["Filesystem      Size  Used Avail Use% Mounted on".to_owned()];
    for idx in 0..16 {
        rows.push(format!(
            "/dev/mapper/vg{}  {}G   {}G   {}G  {}% /mnt/vol{}",
            idx,
            80 + idx,
            20 + idx,
            60,
            20 + (idx % 70),
            idx
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_psql_table() -> String {
    let mut rows = vec![
        " schema |           table            | rows ".to_owned(),
        "--------+----------------------------+------".to_owned(),
    ];
    for idx in 0..18 {
        rows.push(format!(
            " public | table_{idx:02}                 | {} ",
            1000 + idx * 137
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_python_log() -> String {
    let mut rows = Vec::new();
    for idx in 0..120 {
        rows.push(format!("python: step {idx:03} finished"));
    }
    rows.push("warning: deprecated config key".to_owned());
    rows.push("error: script failed at stage 007".to_owned());
    rows.join("\n")
}

pub(crate) fn repeated_benchmark_python_table() -> String {
    let mut rows = vec!["name        count   value".to_owned()];
    for idx in 0..18 {
        rows.push(format!(
            "item_{idx:02}    {:>5}   state_{:02}",
            100 + idx * 11,
            idx
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_lines(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|idx| format!("{prefix} // line {idx}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn repeated_task_search_output(
    highlights: &[&str],
    total_lines: usize,
    path: &str,
    symbol: &str,
) -> String {
    let mut rows = highlights
        .iter()
        .map(|line| (*line).to_owned())
        .collect::<Vec<_>>();
    while rows.len() < total_lines {
        let idx = rows.len() + 1;
        rows.push(format!(
            "{path}:{}:fn {symbol}_{idx:03}() {{ let value = {}; }}",
            2400 + idx,
            idx
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_task_code_block(lines: &[&str], repeats: usize) -> String {
    let mut rows = Vec::new();
    for _ in 0..repeats {
        rows.extend(lines.iter().map(|line| (*line).to_owned()));
    }
    rows.join("\n")
}

pub(crate) fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    usize::max(1, chars.div_ceil(4))
}
