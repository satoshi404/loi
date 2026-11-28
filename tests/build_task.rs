use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_lib() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib")
}

fn test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "loi-build-task-{}-{name}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn write(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent should be creatable");
    }
    fs::write(&path, source).expect("source should be writable");
    path
}

fn has_c_compiler() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn loi_string(value: &str) -> String {
    let mut out = String::from("\"");
    for byte in value.bytes() {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            other => out.push(other as char),
        }
    }
    out.push('"');
    out
}

fn compile(dir: &Path, body: &str) -> PathBuf {
    let script = format!(
        "import build::model\n\
         import build::exec\n\
         \n\
         open flow main\n\
             => i32\n\
         {{\n\
             protect {{\n\
                 mut project: model::Project\n\
                 model::project(\"demo\", -> project)\n\
         {body}\
             }}\n\
         }}\n"
    );
    let check = write(dir, "check.loi", &script);
    let runner = dir.join("runner");
    let output = Command::new(loi())
        .arg("build")
        .arg(&check)
        .arg("-o")
        .arg(&runner)
        .arg("--auto-c-source")
        .arg("--c-source")
        .arg(repo_lib().join("os_shim.c"))
        .arg("--c-source")
        .arg(repo_lib().join("process_shim.c"))
        .arg("--lib-dir")
        .arg(repo_lib())
        .current_dir(dir)
        .output()
        .expect("loi build should run");
    if !output.status.success() {
        panic!(
            "task script failed to compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    runner
}

fn run(runner: &Path, dir: &Path) -> Output {
    Command::new(runner)
        .current_dir(dir)
        .output()
        .expect("runner should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn generated_source(exit_code: &str) -> String {
    format!("open flow main\n    => i32\n{{\n    => {exit_code}\n}}\n")
}

fn write_task_decls(content: &str, output_path: &str, condition: &str) -> String {
    format!(
        "                mut gen: u64\n\
                         model::task(<-> project, \"gen\", \"generate\", \"generate\", -> gen)\n\
                         model::set_task_action(<-> project, gen, \"write\")\n\
                         model::set_task_content(<-> project, gen, {})\n\
                         model::set_task_condition(<-> project, gen, {})\n\
                         model::add_task_output(<-> project, gen, {})\n",
        loi_string(content),
        loi_string(condition),
        loi_string(output_path)
    )
}

fn run_project_line() -> String {
    format!(
        "                => exec::run_project(project, \"{}\")\n",
        loi()
    )
}

#[test]
fn write_task_generates_a_source_that_a_target_compiles() {
    if !has_c_compiler() {
        eprintln!("skipping write_task_generates_a_source_that_a_target_compiles: cc not found");
        return;
    }
    let dir = test_dir("write-source");
    let source = generated_source("7");
    let runner = compile(
        &dir,
        &format!(
            "{}                mut app: u64\n\
                         model::executable(<-> project, \"app\", -> app)\n\
                         model::set_output(<-> project, app, \"output/app\")\n\
                         model::add_source(<-> project, app, \"output/generated/app.loi\")\n\
                         model::add_task_ref(<-> project, app, \"gen\")\n\
                         {}\n",
            write_task_decls(&source, "output/generated/app.loi", "changed"),
            run_project_line()
        ),
    );
    let output = run(&runner, &dir);
    let generated = fs::read_to_string(dir.join("output/generated/app.loi")).unwrap_or_default();
    let binary = dir.join("output/app").exists();
    let manifest = dir.join("output/.loi-build/gen.cache").exists();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(generated, source, "generated source differs");
    assert!(binary, "target did not build from the generated source");
    assert!(manifest, "task manifest was not written");
}

#[test]
fn write_task_is_skipped_while_its_inputs_and_output_are_fresh() {
    if !has_c_compiler() {
        eprintln!(
            "skipping write_task_is_skipped_while_its_inputs_and_output_are_fresh: cc not found"
        );
        return;
    }
    let dir = test_dir("write-skip");
    let runner = compile(
        &dir,
        &format!(
            "{}{}\n",
            write_task_decls("first\n", "output/generated.txt", "changed"),
            run_project_line()
        ),
    );
    let first = run(&runner, &dir);
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    fs::write(dir.join("output/generated.txt"), "sentinel\n").expect("output should be writable");
    let second = run(&runner, &dir);
    let text = fs::read_to_string(dir.join("output/generated.txt")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert_eq!(text, "sentinel\n", "fresh task output was rewritten");
}

#[test]
fn condition_always_rewrites_the_output() {
    if !has_c_compiler() {
        eprintln!("skipping condition_always_rewrites_the_output: cc not found");
        return;
    }
    let dir = test_dir("always");
    let runner = compile(
        &dir,
        &format!(
            "{}{}\n",
            write_task_decls("generated\n", "output/generated.txt", "always"),
            run_project_line()
        ),
    );
    let first = run(&runner, &dir);
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    fs::write(dir.join("output/generated.txt"), "sentinel\n").expect("output should be writable");
    let second = run(&runner, &dir);
    let text = fs::read_to_string(dir.join("output/generated.txt")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert_eq!(
        text, "generated\n",
        "always task did not rewrite its output"
    );
}

#[test]
fn copy_task_copies_a_declared_input() {
    if !has_c_compiler() {
        eprintln!("skipping copy_task_copies_a_declared_input: cc not found");
        return;
    }
    let dir = test_dir("copy");
    write(&dir, "assets/data.bin", "abc\n");
    let body = "                mut gen: u64\n\
                         model::task(<-> project, \"gen\", \"generate\", \"generate\", -> gen)\n\
                         model::set_task_action(<-> project, gen, \"copy\")\n\
                         model::add_task_input(<-> project, gen, \"assets/data.bin\")\n\
                         model::add_task_output(<-> project, gen, \"output/data.bin\")\n\
                         => exec::run_project(project, \"PLACEHOLDER\")\n"
        .replace("PLACEHOLDER", loi());
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = fs::read_to_string(dir.join("output/data.bin")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "abc\n", "copy produced different bytes");
}

#[test]
fn concat_task_joins_declared_inputs_in_order() {
    if !has_c_compiler() {
        eprintln!("skipping concat_task_joins_declared_inputs_in_order: cc not found");
        return;
    }
    let dir = test_dir("concat");
    write(&dir, "parts/a.txt", "one");
    write(&dir, "parts/b.txt", "two");
    let body = "                mut gen: u64\n\
                         model::task(<-> project, \"gen\", \"generate\", \"generate\", -> gen)\n\
                         model::set_task_action(<-> project, gen, \"concat\")\n\
                         model::add_task_input(<-> project, gen, \"parts/a.txt\")\n\
                         model::add_task_input(<-> project, gen, \"parts/b.txt\")\n\
                         model::add_task_output(<-> project, gen, \"output/joined.txt\")\n\
                         => exec::run_project(project, \"PLACEHOLDER\")\n"
        .replace("PLACEHOLDER", loi());
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = fs::read_to_string(dir.join("output/joined.txt")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "one\ntwo", "concat order or separator differs");
}

#[test]
fn a_task_using_another_tasks_output_runs_after_it() {
    if !has_c_compiler() {
        eprintln!("skipping a_task_using_another_tasks_output_runs_after_it: cc not found");
        return;
    }
    let dir = test_dir("task-chain");
    let body = format!(
        "                mut first: u64\n\
                         model::task(<-> project, \"first\", \"generate\", \"generate\", -> first)\n\
                         model::set_task_action(<-> project, first, \"write\")\n\
                         model::set_task_content(<-> project, first, \"base\")\n\
                         model::add_task_output(<-> project, first, \"output/base.txt\")\n\
                         mut second: u64\n\
                         model::task(<-> project, \"second\", \"generate\", \"generate\", -> second)\n\
                         model::set_task_action(<-> project, second, \"concat\")\n\
                         model::add_task_input(<-> project, second, \"output/base.txt\")\n\
                         model::add_task_output(<-> project, second, \"output/final.txt\")\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = fs::read_to_string(dir.join("output/final.txt")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "base", "dependent task ran before its input existed");
}

#[test]
fn duplicate_task_outputs_are_rejected_without_writing() {
    if !has_c_compiler() {
        eprintln!("skipping duplicate_task_outputs_are_rejected_without_writing: cc not found");
        return;
    }
    let dir = test_dir("duplicate-output");
    let body = format!(
        "                mut first: u64\n\
                         model::task(<-> project, \"first\", \"generate\", \"generate\", -> first)\n\
                         model::set_task_action(<-> project, first, \"write\")\n\
                         model::set_task_content(<-> project, first, \"a\")\n\
                         model::add_task_output(<-> project, first, \"output/same.txt\")\n\
                         mut second: u64\n\
                         model::task(<-> project, \"second\", \"generate\", \"generate\", -> second)\n\
                         model::set_task_action(<-> project, second, \"write\")\n\
                         model::set_task_content(<-> project, second, \"b\")\n\
                         model::add_task_output(<-> project, second, \"output/same.txt\")\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let written = dir.join("output/same.txt").exists();
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "collision should be rejected");
    assert!(text.contains("duplicate output"), "unexpected: {text}");
    assert!(!written, "validation wrote before rejecting the collision");
}

#[test]
fn unsupported_task_action_is_rejected_without_shell_execution() {
    if !has_c_compiler() {
        eprintln!(
            "skipping unsupported_task_action_is_rejected_without_shell_execution: cc not found"
        );
        return;
    }
    let dir = test_dir("shell-action");
    let marker = dir.join("sentinel");
    write(&dir, "sentinel", "keep\n");
    let body = format!(
        "                mut gen: u64\n\
                         model::task(<-> project, \"gen\", \"generate\", \"generate\", -> gen)\n\
                         model::set_task_action(<-> project, gen, \"shell\")\n\
                         model::set_task_content(<-> project, gen, \"{}\")\n\
                         model::add_task_output(<-> project, gen, \"output/generated.txt\")\n\
                         => exec::run_project(project, \"{}\")\n",
        marker.display(),
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let kept = fs::read_to_string(&marker).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "shell action should be rejected");
    assert!(text.contains("task action"), "unexpected: {text}");
    assert_eq!(
        kept, "keep\n",
        "arbitrary command touched an undeclared file"
    );
}

#[test]
fn copy_task_with_a_missing_non_generated_input_is_rejected() {
    if !has_c_compiler() {
        eprintln!(
            "skipping copy_task_with_a_missing_non_generated_input_is_rejected: cc not found"
        );
        return;
    }
    let dir = test_dir("missing-input");
    let body = format!(
        "                mut gen: u64\n\
                         model::task(<-> project, \"gen\", \"generate\", \"generate\", -> gen)\n\
                         model::set_task_action(<-> project, gen, \"copy\")\n\
                         model::add_task_input(<-> project, gen, \"missing.txt\")\n\
                         model::add_task_output(<-> project, gen, \"output/copied.txt\")\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "missing input should be rejected");
    assert!(text.contains("missing input"), "unexpected: {text}");
}
