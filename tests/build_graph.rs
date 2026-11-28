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
        "loi-build-graph-{}-{name}-{nanos}",
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

fn compile(dir: &Path, script: &Path) -> Option<PathBuf> {
    let runner = dir.join("runner");
    let output = Command::new(loi())
        .arg("build")
        .arg(script)
        .arg("-o")
        .arg(&runner)
        .arg("--auto-c-source")
        .arg("--lib-dir")
        .arg(repo_lib())
        .current_dir(dir)
        .output()
        .expect("loi build should run");
    if !output.status.success() {
        panic!(
            "graph script failed to compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Some(runner)
}

fn run(runner: &Path, dir: &Path) -> Output {
    Command::new(runner)
        .current_dir(dir)
        .output()
        .expect("runner should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn script(body: &str) -> String {
    format!(
        "import build::model\n\
         import build::graph\n\
         import build::order\n\
         \n\
         open flow main\n\
             => i32\n\
         {{\n\
             protect {{\n\
                 mut project: model::Project\n\
                 model::project(\"demo\", -> project)\n\
         {body}\
                 => 0\n\
             }}\n\
         }}\n"
    )
}

fn source(name: &str) -> String {
    format!("open flow main\n    => i32\n{{\n    => 0\n}}\n// {name}\n")
}

fn print_order(body: &str) -> String {
    script(&format!(
        "{body}\
                 mut ids: Vec<[u8]>\n\
                 order::order_project(project, -> ids)\n\
                 for i in 0..ids.len {{\n\
                     print(vec_get<[u8]>(ids, i))\n\
                     print(\"\\n\")\n\
                 }}\n"
    ))
}

#[test]
fn graph_orders_a_single_target() {
    if !has_c_compiler() {
        eprintln!("skipping graph_orders_a_single_target: cc not found");
        return;
    }
    let dir = test_dir("single");
    write(&dir, "src/main.loi", &source("app"));
    let check = write(
        &dir,
        "check.loi",
        &print_order(
            "                mut app: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::add_source(<-> project, app, \"src/main.loi\")\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "app\n", "unexpected order: {text}");
}

#[test]
fn graph_orders_independent_targets_by_declaration() {
    if !has_c_compiler() {
        eprintln!("skipping graph_orders_independent_targets_by_declaration: cc not found");
        return;
    }
    let dir = test_dir("declaration-order");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/core.loi", &source("core"));
    let check = write(
        &dir,
        "check.loi",
        &print_order(
            "                mut app: u64\n\
                             mut core: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::static_library(<-> project, \"core\", -> core)\n\
                             model::add_source(<-> project, app, \"src/app.loi\")\n\
                             model::add_source(<-> project, core, \"src/core.loi\")\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "app\ncore\n", "unexpected order: {text}");
}

#[test]
fn graph_orders_dependencies_before_dependents() {
    if !has_c_compiler() {
        eprintln!("skipping graph_orders_dependencies_before_dependents: cc not found");
        return;
    }
    let dir = test_dir("dependencies");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/core.loi", &source("core"));
    let check = write(
        &dir,
        "check.loi",
        &print_order(
            "                mut app: u64\n\
                             mut core: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::static_library(<-> project, \"core\", -> core)\n\
                             model::add_source(<-> project, app, \"src/app.loi\")\n\
                             model::add_source(<-> project, core, \"src/core.loi\")\n\
                             model::add_dep(<-> project, app, \"core\")\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "core\napp\n", "unexpected order: {text}");
}

#[test]
fn graph_rejects_a_dependency_cycle() {
    if !has_c_compiler() {
        eprintln!("skipping graph_rejects_a_dependency_cycle: cc not found");
        return;
    }
    let dir = test_dir("cycle");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/core.loi", &source("core"));
    let check = write(
        &dir,
        "check.loi",
        &script(
            "                mut app: u64\n\
                             mut core: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::static_library(<-> project, \"core\", -> core)\n\
                             model::add_source(<-> project, app, \"src/app.loi\")\n\
                             model::add_source(<-> project, core, \"src/core.loi\")\n\
                             model::add_dep(<-> project, app, \"core\")\n\
                             model::add_dep(<-> project, core, \"app\")\n\
                             mut ids: Vec<[u8]>\n\
                             order::order_project(project, -> ids)\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(1), "unexpected exit: {text}");
    assert!(text.contains("dependency cycle"), "unexpected: {text}");
}

#[test]
fn graph_deduplicates_repeated_sources() {
    if !has_c_compiler() {
        eprintln!("skipping graph_deduplicates_repeated_sources: cc not found");
        return;
    }
    let dir = test_dir("duplicate-source");
    write(&dir, "src/main.loi", &source("app"));
    let check = write(
        &dir,
        "check.loi",
        &script(
            "                mut app: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::add_source(<-> project, app, \"src/main.loi\")\n\
                             model::add_source(<-> project, app, \"src/main.loi\")\n\
                             mut paths: Vec<[u8]>\n\
                             graph::target_sources(project, \"app\", -> paths)\n\
                             print(u64_to_bytes(paths.len))\n\
                             print(\"\\n\")\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "1\n", "unexpected source count: {text}");
}

#[test]
fn graph_deduplicates_repeated_edges() {
    if !has_c_compiler() {
        eprintln!("skipping graph_deduplicates_repeated_edges: cc not found");
        return;
    }
    let dir = test_dir("duplicate-edge");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/core.loi", &source("core"));
    let check = write(
        &dir,
        "check.loi",
        &script(
            "                mut app: u64\n\
                             mut core: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::static_library(<-> project, \"core\", -> core)\n\
                             model::add_source(<-> project, app, \"src/app.loi\")\n\
                             model::add_source(<-> project, core, \"src/core.loi\")\n\
                             model::add_dep(<-> project, app, \"core\")\n\
                             model::add_dep(<-> project, app, \"core\")\n\
                             mut edges: graph::BuildGraph\n\
                             graph::build_graph(project, -> edges)\n\
                             print(u64_to_bytes(graph::edge_count(edges)))\n\
                             print(\"\\n\")\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "1\n", "unexpected edge count: {text}");
}

#[test]
fn graph_rejects_duplicate_ids_across_node_kinds() {
    if !has_c_compiler() {
        eprintln!("skipping graph_rejects_duplicate_ids_across_node_kinds: cc not found");
        return;
    }
    let dir = test_dir("duplicate-node");
    write(&dir, "src/main.loi", &source("app"));
    let check = write(
        &dir,
        "check.loi",
        &script(
            "                mut app: u64\n\
                             mut task: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::task(<-> project, \"app\", \"generate\", \"noop\", -> task)\n\
                             model::add_source(<-> project, app, \"src/main.loi\")\n\
                             mut ids: Vec<[u8]>\n\
                             order::order_project(project, -> ids)\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(1), "unexpected exit: {text}");
    assert!(text.contains("duplicate node 'app'"), "unexpected: {text}");
}

#[test]
fn graph_orders_tasks_before_dependent_targets() {
    if !has_c_compiler() {
        eprintln!("skipping graph_orders_tasks_before_dependent_targets: cc not found");
        return;
    }
    let dir = test_dir("task-order");
    write(&dir, "src/app.loi", &source("app"));
    let check = write(
        &dir,
        "check.loi",
        &script(
            "                mut app: u64\n\
                             mut gen: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::task(<-> project, \"gen\", \"generate\", \"noop\", -> gen)\n\
                             model::add_source(<-> project, app, \"src/app.loi\")\n\
                             model::add_task_ref(<-> project, app, \"gen\")\n\
                             mut ids: Vec<[u8]>\n\
                             order::order_project(project, -> ids)\n\
                             for i in 0..ids.len {\n\
                                 print(vec_get<[u8]>(ids, i))\n\
                                 print(\"\\n\")\n\
                             }\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "gen\napp\n", "unexpected order: {text}");
}

#[test]
fn graph_ignores_input_edges_when_ordering() {
    if !has_c_compiler() {
        eprintln!("skipping graph_ignores_input_edges_when_ordering: cc not found");
        return;
    }
    let dir = test_dir("input-edge");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/extra.loi", &source("extra"));
    let check = write(
        &dir,
        "check.loi",
        &script(
            "                mut app: u64\n\
                             mut edge: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::add_source(<-> project, app, \"src/app.loi\")\n\
                             model::dependency(<-> project, \"app\", \"src/extra.loi\", \"input\", -> edge)\n\
                             mut ids: Vec<[u8]>\n\
                             order::order_project(project, -> ids)\n\
                             for i in 0..ids.len {\n\
                                 print(vec_get<[u8]>(ids, i))\n\
                                 print(\"\\n\")\n\
                             }\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "app\n", "unexpected order: {text}");
}

#[test]
fn graph_order_is_stable_across_repeated_calls() {
    if !has_c_compiler() {
        eprintln!("skipping graph_order_is_stable_across_repeated_calls: cc not found");
        return;
    }
    let dir = test_dir("stable-order");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/core.loi", &source("core"));
    let check = write(
        &dir,
        "check.loi",
        &script(
            "                mut app: u64\n\
                             mut core: u64\n\
                             model::executable(<-> project, \"app\", -> app)\n\
                             model::static_library(<-> project, \"core\", -> core)\n\
                             model::add_source(<-> project, app, \"src/app.loi\")\n\
                             model::add_source(<-> project, core, \"src/core.loi\")\n\
                             model::add_dep(<-> project, app, \"core\")\n\
                             mut first: Vec<[u8]>\n\
                             mut second: Vec<[u8]>\n\
                             order::order_project(project, -> first)\n\
                             order::order_project(project, -> second)\n\
                             mut left: [u8] <- \"\"\n\
                             mut right: [u8] <- \"\"\n\
                             for i in 0..first.len {\n\
                                 left = str_concat(left, vec_get<[u8]>(first, i))\n\
                             }\n\
                             for i in 0..second.len {\n\
                                 right = str_concat(right, vec_get<[u8]>(second, i))\n\
                             }\n\
                             if not str_eq(left, right) {\n\
                                 panic(\"unstable order\\n\")\n\
                             }\n\
                             print(left)\n\
                             print(\"\\n\")\n",
        ),
    );
    let runner = compile(&dir, &check).expect("runner");
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "coreapp\n", "unexpected order: {text}");
}
