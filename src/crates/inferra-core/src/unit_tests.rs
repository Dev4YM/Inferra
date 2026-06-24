//! Unit tests for inferra-core.

use super::*;
use inferra_storage::{
    initialize_databases, EventsStore, IncidentRecord, IncidentsStore, NewEventRecord,
    StoredFeedback, StoredHypothesis, StoredInferenceGraphSnapshot,
};
use std::fs;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("inferra-core-{name}-{unique}"))
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

macro_rules! process_runtime_evidence {
    ($process_name:expr, $command:expr, $project_path:expr, $cwd:expr, $script:expr, $executable:expr, $framework:expr, $endpoints:expr $(,)?) => {
        ProcessRuntimeEvidence {
            process_name: $process_name,
            command: $command,
            project_path: $project_path,
            cwd: $cwd,
            script: $script,
            executable: $executable,
            framework: $framework,
            endpoints: $endpoints,
        }
    };
}

#[test]
fn process_cpu_is_normalized_to_total_host_share() {
    assert_eq!(normalize_process_cpu_to_host_percent(100.0, 8), 12.5);
    assert_eq!(normalize_process_cpu_to_host_percent(400.0, 8), 50.0);
    assert_eq!(normalize_process_cpu_to_host_percent(1200.0, 8), 100.0);
}

fn event(
    event_id: &str,
    service_id: &str,
    message: &str,
    severity: i64,
    source_type: &str,
    timestamp: &str,
) -> EventRow {
    EventRow {
        event_id: Some(event_id.into()),
        timestamp: Some(timestamp.into()),
        severity: Some(SeverityValue::Level(severity)),
        service_id: Some(service_id.into()),
        message: Some(message.into()),
        summary: None,
        source_ref: Some(inferra_contracts::EventSourceRef {
            source_type: Some(source_type.into()),
        }),
        tags: None,
        trace_id: None,
        span_id: None,
        signal_kind: None,
        deployment_environment: None,
        severity_text: None,
    }
}

#[test]
fn ai_status_from_config_surfaces_model_overrides() {
    let config: TomlValue = r#"
[ai]
enabled = true
provider = "ollama"
model = "gemma"
model_status = "gemma-status"
model_investigate = "gemma-investigate"
"#
    .parse()
    .expect("parse config");

    let status = ai_status_from_config(&config);
    assert_eq!(status.status_model.as_deref(), Some("gemma-status"));
    assert_eq!(
        status.investigate_model.as_deref(),
        Some("gemma-investigate")
    );
}

#[test]
fn discover_projects_honors_workspace_roots_depth_and_limits() {
    let root = temp_dir("workspace-scan");
    let extra_root = root.join("extra-root");
    fs::create_dir_all(root.join("service-a")).expect("create service-a");
    fs::create_dir_all(root.join("service-b")).expect("create service-b");
    fs::create_dir_all(root.join("deep/one/two/three")).expect("create deep service");
    fs::create_dir_all(extra_root.join("service-c")).expect("create extra service");
    fs::write(root.join("service-a/Cargo.toml"), "[package]\nname='a'\n")
        .expect("write cargo marker");
    fs::write(root.join("service-b/package.json"), "{}").expect("write package marker");
    fs::write(
        root.join("deep/one/two/three/pyproject.toml"),
        "[project]\nname='deep'\n",
    )
    .expect("write deep marker");
    fs::write(
        extra_root.join("service-c/go.mod"),
        "module example.com/servicec\n",
    )
    .expect("write go marker");

    let config: TomlValue = r#"
[workspace]
max_depth = 2
max_results = 10
roots = ["extra-root"]
"#
    .parse()
    .expect("parse config");

    let projects = discover_projects(&config, &root);
    assert!(projects
        .iter()
        .any(|project| project.path.contains("service-a")));
    assert!(projects
        .iter()
        .any(|project| project.path.contains("service-b")));
    assert!(projects
        .iter()
        .any(|project| project.path.contains("service-c")));
    assert!(!projects
        .iter()
        .any(|project| project.path.contains("three")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_roots_default_to_strict_config_scope() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir("workspace-roots-strict");
    let base = root.join("config-root");
    let home = root.join("home");
    fs::create_dir_all(&base).expect("create base");
    fs::create_dir_all(home.join("Projects")).expect("create home projects");

    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    let config: TomlValue = "[workspace]\n".parse().expect("parse config");
    let roots = workspace_roots(&config, &base);

    unsafe {
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_userprofile {
            std::env::set_var("USERPROFILE", value);
        } else {
            std::env::remove_var("USERPROFILE");
        }
    }

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], base.canonicalize().expect("canonical base"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_roots_hybrid_mode_includes_home_candidates() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir("workspace-roots-hybrid");
    let base = root.join("config-root");
    let home = root.join("home");
    fs::create_dir_all(&base).expect("create base");
    fs::create_dir_all(home.join("Projects")).expect("create home projects");

    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    let config: TomlValue = "[workspace]\ndiscovery_mode = \"hybrid\"\n"
        .parse()
        .expect("parse config");
    let roots = workspace_roots(&config, &base);

    unsafe {
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_userprofile {
            std::env::set_var("USERPROFILE", value);
        } else {
            std::env::remove_var("USERPROFILE");
        }
    }

    assert!(roots.iter().any(|path| path.ends_with("Projects")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_roots_support_explicit_home_roots_without_hybrid_mode() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir("workspace-roots-explicit-home");
    let base = root.join("config-root");
    let home = root.join("home");
    fs::create_dir_all(&base).expect("create base");
    fs::create_dir_all(home.join("code")).expect("create home code");

    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    let config: TomlValue = "[workspace]\nhome_roots = [\"code\"]\n"
        .parse()
        .expect("parse config");
    let roots = workspace_roots(&config, &base);

    unsafe {
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_userprofile {
            std::env::set_var("USERPROFILE", value);
        } else {
            std::env::remove_var("USERPROFILE");
        }
    }

    assert!(roots.iter().any(|path| path.ends_with("code")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_roots_auto_bootstrap_home_candidates_for_managed_install_roots() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir("workspace-roots-bootstrap-auto");
    let managed_root = root.join("ProgramData");
    let base = managed_root.join("Inferra");
    let home = root.join("home");
    fs::create_dir_all(&base).expect("create base");
    fs::create_dir_all(home.join("Projects")).expect("create home projects");

    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    let old_programdata = std::env::var_os("PROGRAMDATA");
    let old_allusersprofile = std::env::var_os("ALLUSERSPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("PROGRAMDATA", &managed_root);
        std::env::set_var("ALLUSERSPROFILE", &managed_root);
    }

    let config: TomlValue = "[workspace]\n".parse().expect("parse config");
    let roots = workspace_roots(&config, &base);

    unsafe {
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_userprofile {
            std::env::set_var("USERPROFILE", value);
        } else {
            std::env::remove_var("USERPROFILE");
        }
        if let Some(value) = old_programdata {
            std::env::set_var("PROGRAMDATA", value);
        } else {
            std::env::remove_var("PROGRAMDATA");
        }
        if let Some(value) = old_allusersprofile {
            std::env::set_var("ALLUSERSPROFILE", value);
        } else {
            std::env::remove_var("ALLUSERSPROFILE");
        }
    }

    assert!(roots.iter().any(|path| path.ends_with("Projects")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_roots_can_disable_auto_home_bootstrap() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir("workspace-roots-bootstrap-never");
    let managed_root = root.join("ProgramData");
    let base = managed_root.join("Inferra");
    let home = root.join("home");
    fs::create_dir_all(&base).expect("create base");
    fs::create_dir_all(home.join("Projects")).expect("create home projects");

    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    let old_programdata = std::env::var_os("PROGRAMDATA");
    let old_allusersprofile = std::env::var_os("ALLUSERSPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("PROGRAMDATA", &managed_root);
        std::env::set_var("ALLUSERSPROFILE", &managed_root);
    }

    let config: TomlValue = "[workspace]\nhome_bootstrap_mode = \"never\"\n"
        .parse()
        .expect("parse config");
    let roots = workspace_roots(&config, &base);

    unsafe {
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_userprofile {
            std::env::set_var("USERPROFILE", value);
        } else {
            std::env::remove_var("USERPROFILE");
        }
        if let Some(value) = old_programdata {
            std::env::set_var("PROGRAMDATA", value);
        } else {
            std::env::remove_var("PROGRAMDATA");
        }
        if let Some(value) = old_allusersprofile {
            std::env::set_var("ALLUSERSPROFILE", value);
        } else {
            std::env::remove_var("ALLUSERSPROFILE");
        }
    }

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], base.canonicalize().expect("canonical base"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn discover_projects_prefers_framework_marker_over_git() {
    let root = temp_dir("workspace-framework-marker-preferred");
    let flutter = root.join("givity_customer_app");
    fs::create_dir_all(flutter.join(".git")).expect("create git dir");
    fs::write(flutter.join("pubspec.yaml"), "name: givity_customer_app\n").expect("write pubspec");

    let config: TomlValue = "[workspace]\nmax_depth = 3\nmax_results = 10\n"
        .parse()
        .expect("parse config");
    let projects = discover_projects(&config, &root);
    let project = projects
        .iter()
        .find(|project| project.path.contains("givity_customer_app"))
        .expect("flutter project discovered");

    assert_eq!(project.kind, "flutter");
    assert_eq!(project.marker, "pubspec.yaml");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn discover_projects_skips_bare_git_checkout_without_source_evidence() {
    let root = temp_dir("workspace-bare-git-filter");
    let repo = root.join("notes-repo");
    fs::create_dir_all(repo.join(".git")).expect("create git dir");

    let config: TomlValue = "[workspace]\nmax_depth = 3\nmax_results = 10\n"
        .parse()
        .expect("parse config");
    let projects = discover_projects(&config, &root);

    assert!(!projects
        .iter()
        .any(|project| project.path.contains("notes-repo")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn discover_projects_allows_weak_markers_when_policy_is_always() {
    let root = temp_dir("workspace-weak-marker-always");
    let repo = root.join("notes-repo");
    fs::create_dir_all(repo.join(".git")).expect("create git dir");

    let config: TomlValue = "[workspace]\nweak_marker_policy = \"always\"\n"
        .parse()
        .expect("parse config");
    let projects = discover_projects(&config, &root);

    assert!(projects
        .iter()
        .any(|project| project.path.contains("notes-repo") && project.marker == ".git"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_map_includes_project_apps_by_default() {
    let root = temp_dir("workspace-project-apps-default");
    let project = root.join("inferra-app");
    fs::create_dir_all(project.join(".inferra")).expect("create project");
    fs::write(
        project.join("package.json"),
        r#"{"name":"inferra","dependencies":{"next":"latest"}}"#,
    )
    .expect("write package");
    fs::write(
        project.join(".inferra").join("app.toml"),
        "[app]\nname = \"Inferra UI\"\n",
    )
    .expect("write app manifest");

    let paths = Paths {
        config_path: root.join("inferra.toml"),
        data_dir: root.join("data"),
        events_db: root.join("data").join("events.db"),
        incidents_db: root.join("data").join("incidents.db"),
    };
    let config: TomlValue = "[workspace]\nroots = [\"inferra-app\"]\n"
        .parse()
        .expect("parse config");

    let workspace = build_workspace_map(&config, &paths).expect("workspace map");
    let app = workspace
        .runtime_apps
        .iter()
        .find(|app| app.name == "inferra")
        .expect("project app");
    assert_eq!(app.source, "project");
    assert_eq!(app.status.as_deref(), Some("registered"));
    assert_eq!(
        app.app_state
            .as_ref()
            .map(|state| state.observed_by.as_str()),
        Some("workspace")
    );
    assert_eq!(app.display_name.as_deref(), Some("Inferra UI"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_map_can_disable_project_app_synthesis() {
    let root = temp_dir("workspace-project-apps-disabled");
    let project = root.join("inferra-app");
    fs::create_dir_all(&project).expect("create project");
    fs::write(
        project.join("package.json"),
        r#"{"name":"inferra","dependencies":{"next":"latest"}}"#,
    )
    .expect("write package");

    let paths = Paths {
        config_path: root.join("inferra.toml"),
        data_dir: root.join("data"),
        events_db: root.join("data").join("events.db"),
        incidents_db: root.join("data").join("incidents.db"),
    };
    let config: TomlValue =
        "[workspace]\nroots = [\"inferra-app\"]\ninclude_project_apps = false\n"
            .parse()
            .expect("parse config");

    let workspace = build_workspace_map(&config, &paths).expect("workspace map");
    assert!(workspace.runtime_apps.is_empty());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn runtime_apps_map_to_projects_with_manager_confidence() {
    let project = WorkspaceProject {
        path: r"C:\workspace\api".into(),
        kind: "node".into(),
        marker: "package.json".into(),
    };
    let app = WorkspaceRuntimeApp {
        pid: Some(42),
        name: "api".into(),
        display_name: Some("api".into()),
        runtime: "nodejs".into(),
        language: Some("nodejs".into()),
        process_kind: Some("server".into()),
        framework: Some("nextjs".into()),
        libraries: Vec::new(),
        log_hints: vec!["pm2 logs".into()],
        log_sources: Vec::new(),
        app_url: None,
        endpoints: Vec::new(),
        health_endpoint: None,
        app_location: None,
        resources: None,
        app_state: None,
        context_capabilities: Vec::new(),
        app_structure: Vec::new(),
        manager: Some("pm2".into()),
        status: Some("online".into()),
        cwd: Some(r"C:\workspace\api".into()),
        script: Some(r"C:\workspace\api\server.js".into()),
        command: Some("node server.js".into()),
        project_path: Some(project.path.clone()),
        latest_trace_summary: None,
        confidence: 0.95,
        source: "pm2".into(),
        signals: vec![WorkspaceMappingSignal {
            name: "pm2_jlist".into(),
            confidence: 0.95,
            detail: "PM2 reported this app in jlist".into(),
        }],
    };

    let mapping = mapping_from_runtime_app(&app).expect("runtime mapping");
    assert_eq!(mapping.service_id, "api");
    assert_eq!(mapping.project_path, project.path);
    assert_eq!(mapping.source, "pm2");
    assert!(mapping.confidence >= 0.9);
    assert!(mapping
        .signals
        .iter()
        .any(|signal| signal.name == "runtime_app"));
}

#[test]
fn port_from_command_ignores_node_print_flag() {
    assert_eq!(port_from_command("node -p \"process.version\""), None);
    assert_eq!(
        port_from_command("node server.js http://127.0.0.1:20"),
        None
    );
    assert_eq!(port_from_command("node server.js --port 3001"), Some(3001));
    assert_eq!(
        port_from_command("uvicorn app:app --host 0.0.0.0 --port=8000"),
        Some(8000)
    );
    assert_eq!(
        port_from_command("python manage.py runserver 0.0.0.0:9000"),
        Some(9000)
    );
}

#[test]
fn workspace_app_endpoints_normalize_wildcard_hosts_from_commands() {
    let endpoints = workspace_app_endpoints(
        Some("uvicorn app:app --host 0.0.0.0 --port=8000"),
        Some("uvicorn"),
        None,
        None,
    );
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].url, "http://127.0.0.1:8000");
    assert_eq!(endpoints[0].host.as_deref(), Some("127.0.0.1"));
}

#[test]
fn workspace_app_endpoints_support_https_and_ipv6_commands() {
    let endpoints = workspace_app_endpoints(
        Some("vite --host :: --port 5173 --https"),
        Some("vite"),
        None,
        None,
    );
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].url, "https://[::1]:5173");
    assert_eq!(endpoints[0].host.as_deref(), Some("::1"));
    assert_eq!(endpoints[0].protocol, "https");
}

#[test]
fn workspace_app_endpoints_parse_django_runserver_bind_targets() {
    let endpoints = workspace_app_endpoints(
        Some("python manage.py runserver 0.0.0.0:9000"),
        Some("django"),
        None,
        None,
    );
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].url, "http://127.0.0.1:9000");
}

#[test]
fn workspace_app_endpoints_ignore_hostname_as_bind_host() {
    let env = serde_json::json!({
        "env": {
            "PORT": "3000",
            "HOSTNAME": "workstation-dev",
        }
    });
    let endpoints = workspace_app_endpoints(None, Some("nextjs"), None, Some(&env));
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].url, "http://127.0.0.1:3000");
}

#[test]
fn process_runtime_apps_without_project_need_real_app_signals() {
    let endpoints = vec![WorkspaceAppEndpoint {
        url: "http://127.0.0.1:3000".into(),
        host: Some("127.0.0.1".into()),
        port: Some(3000),
        protocol: "http".into(),
        source: "command".into(),
        confidence: 0.72,
    }];

    assert!(should_keep_process_runtime_app(&process_runtime_evidence!(
        "node",
        "node C:\\apps\\api\\server.js",
        None,
        None,
        Some(r"C:\apps\api\server.js"),
        Some(r"C:\Program Files\nodejs\node.exe"),
        None,
        &[],
    )));
    assert!(should_keep_process_runtime_app(&process_runtime_evidence!(
        "node",
        "node_modules/.bin/vite",
        None,
        None,
        None,
        Some(r"C:\Program Files\nodejs\node.exe"),
        Some("vite"),
        &[],
    )));
    assert!(should_keep_process_runtime_app(&process_runtime_evidence!(
        "node",
        "node server.js --port 3000",
        None,
        None,
        None,
        Some(r"C:\Program Files\nodejs\node.exe"),
        None,
        &endpoints,
    )));
    assert!(!should_keep_process_runtime_app(
        &process_runtime_evidence!(
            "git",
            "git status --porcelain",
            Some(r"D:\MYFiles\Projects\py\Inferra"),
            Some(r"D:\MYFiles\Projects\py\Inferra"),
            None,
            Some(r"C:\Program Files\Git\mingw64\bin\git.exe"),
            None,
            &[],
        )
    ));
    assert!(!should_keep_process_runtime_app(
        &process_runtime_evidence!(
            "node",
            r#"node C:\Users\User\AppData\Local\nvm\v20.20.0\node_modules\pm2\lib\Daemon.js"#,
            Some(r"D:\MYFiles\Projects\py\Inferra\src\crates\inferra-api"),
            Some(r"D:\MYFiles\Projects\py\Inferra\src\crates\inferra-api"),
            Some(r"C:\Users\User\AppData\Local\nvm\v20.20.0\node_modules\pm2\lib\Daemon.js"),
            Some(r"C:\Users\User\AppData\Local\nvm\v20.20.0\node.exe"),
            None,
            &[],
        )
    ));
    assert!(!should_keep_process_runtime_app(
        &process_runtime_evidence!(
            "npm",
            "npm run dev -- --port 3000",
            Some(r"D:\MYFiles\Projects\py\Inferra\src\web\frontend"),
            Some(r"D:\MYFiles\Projects\py\Inferra\src\web\frontend"),
            Some(r"C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js"),
            Some(r"C:\Program Files\nodejs\npm.cmd"),
            Some("vite"),
            &endpoints,
        )
    ));
    assert!(!should_keep_process_runtime_app(
        &process_runtime_evidence!(
            "powershell",
            "powershell -noexit",
            Some(r"D:\MYFiles\Projects\Server\EECP"),
            Some(r"D:\MYFiles\Projects\Server\EECP"),
            None,
            Some(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            None,
            &[],
        )
    ));
    assert!(!should_keep_process_runtime_app(
        &process_runtime_evidence!(
            "cmd",
            "cmd.exe",
            None,
            None,
            None,
            Some(r"C:\Windows\System32\cmd.exe"),
            None,
            &[],
        )
    ));
    assert!(should_keep_process_runtime_app(&process_runtime_evidence!(
        "cmd",
        "cmd.exe /d /s /c next start -p 81",
        Some(r"D:\MYFiles\Projects\Next.js\main"),
        Some(r"D:\MYFiles\Projects\Next.js\main"),
        Some("next"),
        Some(r"C:\Windows\System32\cmd.exe"),
        Some("next"),
        &[WorkspaceAppEndpoint {
            url: "http://127.0.0.1:81".into(),
            host: Some("127.0.0.1".into()),
            port: Some(81),
            protocol: "http".into(),
            source: "command".into(),
            confidence: 0.72,
        }],
    )));
    assert!(!should_keep_process_runtime_app(
        &process_runtime_evidence!(
            "powershell",
            "powershell -File D:\\MYFiles\\Projects\\py\\Inferra\\scripts\\serve.ps1",
            Some(r"D:\MYFiles\Projects\py\Inferra"),
            Some(r"D:\MYFiles\Projects\py\Inferra"),
            Some(r"D:\MYFiles\Projects\py\Inferra\scripts\serve.ps1"),
            Some(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            None,
            &[],
        )
    ));
}

#[test]
fn workspace_manifest_enriches_runtime_app_context() {
    let root = temp_dir("workspace-manifest");
    let app_dir = root.join("billing-api");
    fs::create_dir_all(app_dir.join(".inferra")).expect("create inferra dir");
    fs::create_dir_all(app_dir.join("logs")).expect("create logs dir");
    fs::write(
        app_dir.join("package.json"),
        r#"{"dependencies":{"fastify":"latest"}}"#,
    )
    .expect("write package");
    fs::write(app_dir.join("logs/app.log"), "server started\n").expect("write log");
    fs::write(
        app_dir.join(".inferra/app.toml"),
        r#"
[app]
name = "Billing API"
url = "http://127.0.0.1:3001"
framework = "fastify"

[heartbeat]
path = "/health"

[[logs]]
label = "App log"
path = "logs/app.log"
kind = "file"
"#,
    )
    .expect("write manifest");

    let mut app = WorkspaceRuntimeApp {
        pid: None,
        name: "billing-api".into(),
        display_name: None,
        runtime: "nodejs".into(),
        language: Some("nodejs".into()),
        process_kind: Some("server".into()),
        framework: None,
        libraries: Vec::new(),
        log_hints: Vec::new(),
        log_sources: Vec::new(),
        app_url: None,
        endpoints: Vec::new(),
        health_endpoint: None,
        app_location: None,
        resources: None,
        app_state: None,
        context_capabilities: Vec::new(),
        app_structure: Vec::new(),
        manager: None,
        status: None,
        cwd: Some(display_path(&app_dir)),
        script: None,
        command: None,
        project_path: Some(display_path(&app_dir)),
        latest_trace_summary: None,
        confidence: 0.8,
        source: "process".into(),
        signals: Vec::new(),
    };

    apply_workspace_manifest(&mut app);

    assert_eq!(app.display_name.as_deref(), Some("Billing API"));
    assert_eq!(app.framework.as_deref(), Some("fastify"));
    assert_eq!(app.app_url.as_deref(), Some("http://127.0.0.1:3001"));
    assert_eq!(
        app.health_endpoint
            .as_ref()
            .map(|endpoint| endpoint.url.as_str()),
        Some("http://127.0.0.1:3001/health")
    );
    assert!(app.log_sources.iter().any(|source| {
        source.label == "App log"
            && source.path.as_deref().is_some_and(|path| {
                path.ends_with(r"logs\app.log") || path.ends_with("logs/app.log")
            })
    }));
    assert!(app
        .signals
        .iter()
        .any(|signal| signal.name == "inferra_manifest"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn project_log_discovery_finds_nested_and_rotated_logs() {
    let root = temp_dir("workspace-log-discovery");
    let app_dir = root.join("api");
    fs::create_dir_all(app_dir.join("var/runtime/deep")).expect("create nested logs");
    fs::create_dir_all(app_dir.join(".next/diagnostics")).expect("create next diagnostics");
    fs::write(
        app_dir.join("package.json"),
        r#"{"dependencies":{"express":"latest"}}"#,
    )
    .expect("write package");
    fs::write(app_dir.join("var/runtime/deep/server.log.1"), "old\n").expect("write rotated log");
    fs::write(app_dir.join("events-log.jsonl"), "{}\n").expect("write jsonl log");
    fs::write(app_dir.join(".next/trace"), "{}\n").expect("write next trace");

    let logs = discover_project_log_files(
        &display_path(&app_dir),
        "nodejs",
        Some("express"),
        &[],
        None,
    );

    assert!(logs.iter().any(|path| path.contains("server.log.1")));
    assert!(logs.iter().any(|path| path.contains("events-log.jsonl")));
    assert!(logs
        .iter()
        .any(|path| path.contains(".next") && path.ends_with("trace")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn npm_cache_logs_are_discovered_for_node_apps() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir("workspace-npm-cache-logs");
    let cache = root.join("npm-cache");
    fs::create_dir_all(cache.join("_logs")).expect("create npm logs");
    fs::write(
        cache.join("_logs/2026-05-14T08_39_16_795Z-debug-0.log"),
        "npm start\n",
    )
    .expect("write npm debug log");

    let old_cache = std::env::var_os("NPM_CONFIG_CACHE");
    let old_local = std::env::var_os("LOCALAPPDATA");
    let old_appdata = std::env::var_os("APPDATA");
    unsafe {
        std::env::set_var("NPM_CONFIG_CACHE", &cache);
        std::env::remove_var("LOCALAPPDATA");
        std::env::remove_var("APPDATA");
    }

    let sources = workspace_log_sources(WorkspaceLogSourceInput {
        manager: None,
        runtime: "nodejs",
        framework: Some("nextjs"),
        libraries: &[],
        project_path: Some(&display_path(&root)),
        cwd: Some(&display_path(&root)),
        script: None,
        pm2_env: None,
    });

    unsafe {
        if let Some(value) = old_cache {
            std::env::set_var("NPM_CONFIG_CACHE", value);
        } else {
            std::env::remove_var("NPM_CONFIG_CACHE");
        }
        if let Some(value) = old_local {
            std::env::set_var("LOCALAPPDATA", value);
        } else {
            std::env::remove_var("LOCALAPPDATA");
        }
        if let Some(value) = old_appdata {
            std::env::set_var("APPDATA", value);
        } else {
            std::env::remove_var("APPDATA");
        }
    }

    assert!(sources.iter().any(|source| {
        source.source == "npm_cache"
            && source
                .path
                .as_deref()
                .is_some_and(|path| path.contains("-debug-0.log"))
    }));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_log_sources_include_pm2_home_files() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir("workspace-pm2-home");
    let home = root.join("home");
    let project = root.join("apps/api");
    fs::create_dir_all(home.join(".pm2/logs")).expect("create pm2 logs");
    fs::create_dir_all(&project).expect("create project");
    fs::write(home.join(".pm2/logs/api-out.log"), "online\n").expect("write pm2 out");

    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    let sources = workspace_log_sources(WorkspaceLogSourceInput {
        manager: Some("pm2"),
        runtime: "nodejs",
        framework: Some("express"),
        libraries: &[],
        project_path: Some(&display_path(&project)),
        cwd: Some(&display_path(&project)),
        script: Some(&display_path(&project.join("server.js"))),
        pm2_env: None,
    });

    unsafe {
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_userprofile {
            std::env::set_var("USERPROFILE", value);
        } else {
            std::env::remove_var("USERPROFILE");
        }
    }

    assert!(sources.iter().any(|source| {
        source.source == "pm2_home"
            && source
                .path
                .as_deref()
                .is_some_and(|path| path.contains("api-out.log"))
    }));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_project_acceptance_skips_installed_packages_unless_registered() {
    let root = temp_dir("workspace-installed-package-filter");
    let package = root.join("node_modules/pm2");
    fs::create_dir_all(&package).expect("create package");
    fs::write(package.join("package.json"), "{}").expect("write package marker");

    assert!(!should_accept_workspace_project(&package));
    fs::create_dir_all(package.join(".inferra")).expect("create inferra dir");
    fs::write(
        package.join(".inferra/app.toml"),
        "[app]\nname='registered'\n",
    )
    .expect("write manifest");
    assert!(should_accept_workspace_project(&package));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_project_acceptance_rejects_user_profile_roots() {
    assert!(!should_accept_workspace_project(Path::new(
        r"C:\Users\User"
    )));
    assert!(!should_accept_workspace_project(Path::new("/Users/user")));
    assert!(!should_accept_workspace_project(Path::new("/home/user")));
}

#[test]
fn dependency_scan_discovers_framework_log_files_without_manifest() {
    let root = temp_dir("workspace-dependency-log-scan");
    fs::create_dir_all(root.join("logs")).expect("create logs");
    fs::write(
        root.join("package.json"),
        r#"{"dependencies":{"fastify":"latest","pino":"latest"}}"#,
    )
    .expect("write package");
    fs::write(root.join("logs/app.log"), "ready\n").expect("write log");
    let project_path = display_path(&root);
    let mut app = WorkspaceRuntimeApp {
        pid: None,
        name: "api".into(),
        display_name: None,
        runtime: "nodejs".into(),
        language: Some("nodejs".into()),
        process_kind: Some("server".into()),
        framework: None,
        libraries: Vec::new(),
        log_hints: Vec::new(),
        log_sources: Vec::new(),
        app_url: None,
        endpoints: Vec::new(),
        health_endpoint: None,
        app_location: None,
        resources: None,
        app_state: None,
        context_capabilities: Vec::new(),
        app_structure: Vec::new(),
        manager: None,
        status: None,
        cwd: Some(project_path.clone()),
        script: None,
        command: None,
        project_path: Some(project_path),
        latest_trace_summary: None,
        confidence: 0.8,
        source: "process".into(),
        signals: Vec::new(),
    };

    apply_workspace_manifest(&mut app);

    assert_eq!(app.framework.as_deref(), Some("fastify"));
    assert!(app.libraries.iter().any(|library| library == "pino"));
    assert!(app.log_sources.iter().any(|source| {
        source.kind == "file"
            && source.path.as_deref().is_some_and(|path| {
                path.ends_with("logs/app.log") || path.ends_with(r"logs\app.log")
            })
    }));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn service_token_mapping_keeps_low_confidence_fallback_available() {
    let projects = vec![WorkspaceProject {
        path: "/srv/projects/billing-api".into(),
        kind: "python".into(),
        marker: "pyproject.toml".into(),
    }];

    let mapping = mapping_from_service_tokens("billing-api", &projects).expect("token mapping");
    assert_eq!(mapping.project_path, "/srv/projects/billing-api");
    assert_eq!(mapping.source, "auto");
    assert!(mapping.confidence >= 0.45);
}

#[test]
fn project_for_paths_discovers_project_root_outside_configured_scan_roots() {
    let root = temp_dir("runtime-project-root");
    let app = root.join("apps").join("web");
    let src = app.join("src");
    fs::create_dir_all(&src).expect("create app src");
    fs::write(app.join("package.json"), r#"{"name":"web-app"}"#).expect("write package");
    let script = src.join("server.js");
    fs::write(&script, "console.log('ok')").expect("write script");

    let locator = WorkspaceProjectLocator {
        projects: &[],
        allowed_roots: &[],
        allow_unscoped_resolution: true,
    };
    let discovered = project_for_paths(
        &locator,
        Some(src.to_string_lossy().as_ref()),
        Some(script.to_string_lossy().as_ref()),
    )
    .expect("discover project from runtime path");
    assert_eq!(
        discovered,
        clean_display_path(&app.canonicalize().expect("canonical app").to_string_lossy())
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn project_for_paths_stays_within_allowed_roots_by_default() {
    let root = temp_dir("runtime-project-root-scoped");
    let allowed = root.join("allowed");
    let outside = root.join("outside");
    let src = outside.join("apps").join("web").join("src");
    fs::create_dir_all(&allowed).expect("create allowed root");
    fs::create_dir_all(&src).expect("create app src");
    let app = src
        .parent()
        .and_then(|path| path.parent())
        .expect("app root");
    fs::write(app.join("package.json"), r#"{"name":"web-app"}"#).expect("write package");
    let script = src.join("server.js");
    fs::write(&script, "console.log('ok')").expect("write script");

    let allowed_roots = vec![allowed.canonicalize().expect("canonical allowed")];
    let locator = WorkspaceProjectLocator {
        projects: &[],
        allowed_roots: &allowed_roots,
        allow_unscoped_resolution: false,
    };
    let discovered = project_for_paths(
        &locator,
        Some(src.to_string_lossy().as_ref()),
        Some(script.to_string_lossy().as_ref()),
    );

    assert!(discovered.is_none());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn clean_display_path_removes_windows_extended_prefix() {
    assert_eq!(
        clean_display_path(r"\\?\C:\Users\dev\app"),
        r"C:\Users\dev\app"
    );
    assert_eq!(
        clean_display_path(r"\\?\UNC\server\share\app"),
        r"\\server\share\app"
    );
}

#[test]
fn build_hypotheses_honors_custom_rules_and_calibration() {
    let config: TomlValue = r#"
[hypothesis_engine]
max_hypotheses_per_incident = 5
min_supporting_events = 1
min_generation_confidence = 0.1
dedup_overlap_threshold = 0.5

[[hypothesis_engine.custom_rules]]
name = "redis_timeout_cascade"
requires = ["connection_failures_outbound", "error_spike"]
requires_same_service = false
requires_temporal_order = true
cause_type = "dependency_failure"
cause_subtype = "redis_timeout"
title_template = "Redis timeout causing service errors"
confidence = 0.82

[calibration.defaults]
high_threshold = 0.7
medium_threshold = 0.45
"#
    .parse()
    .expect("parse config");
    let events = vec![
        event(
            "e1",
            "api",
            "redis timeout upstream dependency",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:00Z",
        ),
        event(
            "e2",
            "worker",
            "error spike after connection refused",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:10Z",
        ),
    ];
    let graph = build_inference_graph(&config, &events);
    let hypotheses = build_hypotheses(
        &config,
        "inc-1",
        &events,
        "2026-05-08T10:00:15Z",
        &graph,
        &LearningArtifacts::default(),
    );
    let top = hypotheses.first().expect("top hypothesis");
    assert_eq!(top.cause_type, "dependency_failure");
    assert_eq!(top.confidence_label.as_deref(), Some("medium"));
    assert!(top
        .description
        .contains("Redis timeout causing service errors"));
}

#[test]
fn build_hypotheses_marks_contradicted_candidates_invalid() {
    let config: TomlValue = r#"
[hypothesis_engine]
max_hypotheses_per_incident = 5
min_supporting_events = 1
min_generation_confidence = 0.1

[hypothesis_validation]
contradiction_ratio_fail = 0.5
contradiction_ratio_warn = 0.2

[contradiction_handling]
enabled = true
strong_penalty_per_contradiction = 0.2
weak_penalty_per_contradiction = 0.05
min_penalty_multiplier = 0.5
"#
    .parse()
    .expect("parse config");
    let events = vec![
        event(
            "e1",
            "api",
            "cpu saturation and memory pressure",
            SEVERITY_ERROR,
            "host_metrics",
            "2026-05-08T10:00:00Z",
        ),
        event(
            "e2",
            "api",
            "memory normal and resource recovered",
            SEVERITY_INFO,
            "host_metrics",
            "2026-05-08T10:00:05Z",
        ),
    ];
    let graph = build_inference_graph(&config, &events);
    let resource = build_hypotheses(
        &config,
        "inc-2",
        &events,
        "2026-05-08T10:00:10Z",
        &graph,
        &LearningArtifacts::default(),
    )
    .into_iter()
    .find(|item| item.cause_type == "resource_pressure")
    .expect("resource hypothesis");
    assert!(!resource.is_valid);
    assert!(!resource.invalidation_reasons.is_empty());
}

#[test]
fn parse_container_line_extracts_runtime_container() {
    let container =
        parse_container_line("api\tghcr.io/acme/api:1.2\tUp 4 minutes").expect("container line");
    assert_eq!(container.name, "api");
    assert_eq!(container.image, "ghcr.io/acme/api:1.2");
    assert_eq!(container.state, "up");
}

#[test]
fn build_inference_graph_creates_roots_and_edges() {
    let config: TomlValue = r#"
[inference_graph]
max_events_for_graph = 50
plausibility_threshold = 0.05
max_edges_per_node = 10

[inference_graph.strategies]
dependency_propagation = true
same_service_escalation = true
resource_preceded_error = true
config_preceded_error = true
restart_preceded_disconnection = true
shared_fate = true
timeout_chain = true

[[topology.edges]]
source = "api"
target = "postgres"
"#
    .parse()
    .expect("parse config");
    let events = vec![
        event(
            "e1",
            "postgres",
            "postgres restart detected",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:00Z",
        ),
        event(
            "e2",
            "api",
            "connection refused to postgres",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:05Z",
        ),
    ];
    let graph = build_inference_graph(&config, &events);
    assert_eq!(graph.root_candidates, vec!["e1".to_string()]);
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.target_event_id == "e2" && edge.source_event_id == "e1"));
}

#[test]
fn sync_learning_artifacts_updates_calibration_and_weights() {
    let root = temp_dir("learning");
    let events_db = root.join("events.db");
    let incidents_db = root.join("incidents.db");
    initialize_databases(&events_db, &incidents_db).expect("initialize databases");
    let mut incidents = IncidentsStore::open(&incidents_db)
        .expect("open incidents")
        .expect("incidents store");
    incidents
        .upsert_incident(
            &IncidentRecord {
                incident_id: "inc-1".into(),
                state: "open".into(),
                severity: SEVERITY_ERROR,
                primary_service: "api".into(),
                affected_services: vec!["api".into()],
                created_at: "2026-05-08T10:00:00Z".into(),
                updated_at: "2026-05-08T10:00:00Z".into(),
                time_range_start: "2026-05-08T10:00:00Z".into(),
                time_range_end: "2026-05-08T10:00:00Z".into(),
                event_count: 2,
                cluster_ids: Vec::new(),
                runtime_context: None,
                resolution_info: None,
            },
            &[],
        )
        .expect("upsert incident");
    incidents
        .replace_hypotheses(
            "inc-1",
            &[
                StoredHypothesis {
                    hypothesis_id: "hyp-ok".into(),
                    rank: Some(1),
                    cause_type: "dependency_failure".into(),
                    description: "correct".into(),
                    total_score: Some(0.82),
                    score_breakdown: serde_json::json!({
                        "temporal_alignment": 0.9,
                        "correlation_strength": 0.6,
                        "frequency_weight": 0.4,
                        "dependency_proximity": 0.95,
                        "evidence_coverage": 0.9,
                        "anomaly_severity": 0.5
                    }),
                    supporting_events: vec!["e1".into()],
                    contradicting_events: Vec::new(),
                    affected_services: vec!["api".into()],
                    suggested_checks: Vec::new(),
                    confidence_label: Some("medium".into()),
                    is_valid: true,
                    invalidation_reasons: Vec::new(),
                    created_at: "2026-05-08T10:00:00Z".into(),
                    updated_at: "2026-05-08T10:00:00Z".into(),
                },
                StoredHypothesis {
                    hypothesis_id: "hyp-bad".into(),
                    rank: Some(2),
                    cause_type: "unknown".into(),
                    description: "wrong".into(),
                    total_score: Some(0.33),
                    score_breakdown: serde_json::json!({
                        "temporal_alignment": 0.2,
                        "correlation_strength": 0.2,
                        "frequency_weight": 0.1,
                        "dependency_proximity": 0.2,
                        "evidence_coverage": 0.1,
                        "anomaly_severity": 0.2
                    }),
                    supporting_events: vec!["e2".into()],
                    contradicting_events: Vec::new(),
                    affected_services: vec!["api".into()],
                    suggested_checks: Vec::new(),
                    confidence_label: Some("low".into()),
                    is_valid: true,
                    invalidation_reasons: Vec::new(),
                    created_at: "2026-05-08T10:00:00Z".into(),
                    updated_at: "2026-05-08T10:00:00Z".into(),
                },
            ],
        )
        .expect("replace hypotheses");
    incidents
        .add_feedback(&StoredFeedback {
            feedback_id: "fb-1".into(),
            incident_id: "inc-1".into(),
            correct_hypothesis_id: Some("hyp-ok".into()),
            feedback_type: "confirmed".into(),
            operator_notes: "matched".into(),
            resolved_at: "2026-05-08T10:05:00Z".into(),
            created_at: Some("2026-05-08T10:05:00Z".into()),
        })
        .expect("add feedback");
    let config: TomlValue = r#"
[calibration]
enabled = true
bucket_count = 5
min_samples_per_bucket = 1
staleness_threshold_days = 30
persistence_file = "./data/calibration.json"

[scoring.tuning]
learning_rate = 0.1
max_drift_from_default = 0.5
min_weight = 0.03
"#
    .parse()
    .expect("parse config");
    let learning = sync_learning_artifacts(&config, &events_db, &incidents_db, &mut incidents)
        .expect("sync learning artifacts");
    assert_eq!(learning.calibration.total_feedback_count, 1);
    assert!(learning
        .calibration
        .buckets
        .iter()
        .any(|bucket| bucket.total_predictions > 0));
    let default_evidence = scoring_weight_defaults()["evidence_coverage"];
    let learned_evidence = learning.weights.effective_weights["evidence_coverage"];
    assert!(learned_evidence > default_evidence);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sync_learning_artifacts_learns_detector_and_template_from_feedback() {
    let root = temp_dir("adaptive-learning");
    let events_db = root.join("events.db");
    let incidents_db = root.join("incidents.db");
    initialize_databases(&events_db, &incidents_db).expect("initialize databases");
    let mut events = EventsStore::open(&events_db)
        .expect("open events")
        .expect("events store");
    events
        .insert_batch(&[
            NewEventRecord {
                tags: vec!["migration".into(), "postgres".into()],
                ..NewEventRecord::minimal(
                    "e1",
                    "2026-05-08T10:00:00Z",
                    "api",
                    SEVERITY_ERROR,
                    "schema migration applied to postgres and queries started timing out",
                    "app",
                    "2026-05-08T10:00:00Z",
                )
            },
            NewEventRecord {
                tags: vec!["migration".into(), "timeout".into()],
                ..NewEventRecord::minimal(
                    "e2",
                    "2026-05-08T10:00:03Z",
                    "api",
                    SEVERITY_ERROR,
                    "request timeout after schema migration on postgres",
                    "app",
                    "2026-05-08T10:00:03Z",
                )
            },
        ])
        .expect("insert events");
    let mut incidents = IncidentsStore::open(&incidents_db)
        .expect("open incidents")
        .expect("incidents store");
    incidents
        .upsert_incident(
            &IncidentRecord {
                incident_id: "inc-adaptive".into(),
                state: "open".into(),
                severity: SEVERITY_ERROR,
                primary_service: "api".into(),
                affected_services: vec!["api".into()],
                created_at: "2026-05-08T10:00:00Z".into(),
                updated_at: "2026-05-08T10:00:05Z".into(),
                time_range_start: "2026-05-08T10:00:00Z".into(),
                time_range_end: "2026-05-08T10:00:05Z".into(),
                event_count: 2,
                cluster_ids: Vec::new(),
                runtime_context: None,
                resolution_info: None,
            },
            &["e1".into(), "e2".into()],
        )
        .expect("upsert incident");
    incidents
        .replace_hypotheses(
            "inc-adaptive",
            &[StoredHypothesis {
                hypothesis_id: "hyp-migration".into(),
                rank: Some(1),
                cause_type: "migration_regression".into(),
                description: "Schema migration on postgres caused request timeouts".into(),
                total_score: Some(0.91),
                score_breakdown: serde_json::json!({
                    "temporal_alignment": 0.9,
                    "correlation_strength": 0.8,
                    "frequency_weight": 0.4,
                    "dependency_proximity": 0.6,
                    "evidence_coverage": 0.9,
                    "anomaly_severity": 0.7
                }),
                supporting_events: vec!["e1".into(), "e2".into()],
                contradicting_events: Vec::new(),
                affected_services: vec!["api".into()],
                suggested_checks: vec!["Review the migration diff".into()],
                confidence_label: Some("high".into()),
                is_valid: true,
                invalidation_reasons: Vec::new(),
                created_at: "2026-05-08T10:00:05Z".into(),
                updated_at: "2026-05-08T10:00:05Z".into(),
            }],
        )
        .expect("replace hypotheses");
    incidents
        .add_feedback(&StoredFeedback {
            feedback_id: "fb-adaptive".into(),
            incident_id: "inc-adaptive".into(),
            correct_hypothesis_id: Some("hyp-migration".into()),
            feedback_type: "confirmed".into(),
            operator_notes: "the migration really broke postgres callers".into(),
            resolved_at: "2026-05-08T10:06:00Z".into(),
            created_at: Some("2026-05-08T10:06:00Z".into()),
        })
        .expect("add feedback");
    let config: TomlValue = r#"
[hypothesis_engine]
max_hypotheses_per_incident = 5
min_supporting_events = 1
min_generation_confidence = 0.1

[calibration]
enabled = true
bucket_count = 5
min_samples_per_bucket = 1
staleness_threshold_days = 30
persistence_file = "./data/calibration.json"

[scoring.tuning]
learning_rate = 0.1
max_drift_from_default = 0.5
min_weight = 0.03
"#
    .parse()
    .expect("parse config");
    let learning = sync_learning_artifacts(&config, &events_db, &incidents_db, &mut incidents)
        .expect("sync learning artifacts");
    assert!(learning
        .adaptive
        .learned_detectors
        .iter()
        .any(|detector| detector.cause_type == "migration_regression"
            && detector
                .positive_terms
                .iter()
                .any(|term| term == "migration" || term == "postgres")));
    assert!(learning
        .adaptive
        .learned_templates
        .iter()
        .any(|template| template.cause_type == "migration_regression"));

    let new_events = vec![
        event(
            "n1",
            "api",
            "postgres timeout immediately after migration",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T11:00:00Z",
        ),
        event(
            "n2",
            "api",
            "schema migration triggered failing postgres queries",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T11:00:03Z",
        ),
    ];
    let graph = build_inference_graph(&config, &new_events);
    let hypotheses = build_hypotheses(
        &config,
        "inc-adaptive-2",
        &new_events,
        "2026-05-08T11:00:05Z",
        &graph,
        &learning,
    );
    assert!(hypotheses.iter().any(|hypothesis| {
        hypothesis.cause_type == "migration_regression"
            && hypothesis
                .description
                .contains("Schema migration on postgres caused request timeouts")
    }));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sync_learning_artifacts_learns_compositions_and_edge_profiles() {
    let root = temp_dir("adaptive-composition");
    let events_db = root.join("events.db");
    let incidents_db = root.join("incidents.db");
    initialize_databases(&events_db, &incidents_db).expect("initialize databases");
    let mut events = EventsStore::open(&events_db)
        .expect("open events")
        .expect("events store");
    events
        .insert_batch(&[
            NewEventRecord {
                tags: vec!["restart".into(), "postgres".into()],
                ..NewEventRecord::minimal(
                    "e1",
                    "2026-05-08T10:00:00Z",
                    "postgres",
                    SEVERITY_ERROR,
                    "postgres restart panic detected",
                    "app",
                    "2026-05-08T10:00:00Z",
                )
            },
            NewEventRecord {
                tags: vec!["timeout".into(), "postgres".into()],
                ..NewEventRecord::minimal(
                    "e2",
                    "2026-05-08T10:00:05Z",
                    "api",
                    SEVERITY_ERROR,
                    "timeout calling postgres upstream dependency",
                    "app",
                    "2026-05-08T10:00:05Z",
                )
            },
            NewEventRecord {
                tags: vec!["error_spike".into(), "postgres".into()],
                ..NewEventRecord::minimal(
                    "e3",
                    "2026-05-08T10:00:08Z",
                    "api",
                    SEVERITY_ERROR,
                    "error spike after connection refused from postgres",
                    "app",
                    "2026-05-08T10:00:08Z",
                )
            },
        ])
        .expect("insert events");
    let config: TomlValue = r#"
[hypothesis_engine]
max_hypotheses_per_incident = 5
min_supporting_events = 1
min_generation_confidence = 0.1

[inference_graph]
max_events_for_graph = 50
plausibility_threshold = 0.05
max_edges_per_node = 10

[inference_graph.strategies]
dependency_propagation = true
same_service_escalation = true
resource_preceded_error = true
config_preceded_error = true
restart_preceded_disconnection = true
shared_fate = true
timeout_chain = true

[calibration]
enabled = true
bucket_count = 5
min_samples_per_bucket = 1
staleness_threshold_days = 30
persistence_file = "./data/calibration.json"

[scoring.tuning]
learning_rate = 0.1
max_drift_from_default = 0.5
min_weight = 0.03

[[topology.edges]]
source = "api"
target = "postgres"
"#
    .parse()
    .expect("parse config");
    let incident_events = vec![
        event(
            "e1",
            "postgres",
            "postgres restart panic detected",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:00Z",
        ),
        event(
            "e2",
            "api",
            "timeout calling postgres upstream dependency",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:05Z",
        ),
        event(
            "e3",
            "api",
            "error spike after connection refused from postgres",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:08Z",
        ),
    ];
    let graph = build_inference_graph_with_learning(
        &config,
        &incident_events,
        &LearningArtifacts::default(),
    );
    let mut incidents = IncidentsStore::open(&incidents_db)
        .expect("open incidents")
        .expect("incidents store");
    incidents
        .upsert_incident(
            &IncidentRecord {
                incident_id: "inc-comp".into(),
                state: "open".into(),
                severity: SEVERITY_ERROR,
                primary_service: "api".into(),
                affected_services: vec!["api".into(), "postgres".into()],
                created_at: "2026-05-08T10:00:00Z".into(),
                updated_at: "2026-05-08T10:00:08Z".into(),
                time_range_start: "2026-05-08T10:00:00Z".into(),
                time_range_end: "2026-05-08T10:00:08Z".into(),
                event_count: 3,
                cluster_ids: Vec::new(),
                runtime_context: None,
                resolution_info: None,
            },
            &["e1".into(), "e2".into(), "e3".into()],
        )
        .expect("upsert incident");
    incidents
        .upsert_inference_graph_snapshot(&StoredInferenceGraphSnapshot {
            incident_id: "inc-comp".into(),
            graph_data: serde_json::to_value(&graph).expect("serialize graph"),
            created_at: "2026-05-08T10:00:08Z".into(),
            event_count: 3,
        })
        .expect("upsert graph snapshot");
    incidents
        .replace_hypotheses(
            "inc-comp",
            &[StoredHypothesis {
                hypothesis_id: "hyp-comp".into(),
                rank: Some(1),
                cause_type: "dependency_failure".into(),
                description: "Postgres restart triggered upstream timeout cascade".into(),
                total_score: Some(0.89),
                score_breakdown: serde_json::json!({
                    "temporal_alignment": 0.85,
                    "correlation_strength": 0.8,
                    "frequency_weight": 0.5,
                    "dependency_proximity": 0.95,
                    "evidence_coverage": 0.9,
                    "anomaly_severity": 0.55
                }),
                supporting_events: vec!["e1".into(), "e2".into(), "e3".into()],
                contradicting_events: Vec::new(),
                affected_services: vec!["api".into(), "postgres".into()],
                suggested_checks: vec!["Inspect postgres restart cause".into()],
                confidence_label: Some("high".into()),
                is_valid: true,
                invalidation_reasons: Vec::new(),
                created_at: "2026-05-08T10:00:08Z".into(),
                updated_at: "2026-05-08T10:00:08Z".into(),
            }],
        )
        .expect("replace hypotheses");
    incidents
        .add_feedback(&StoredFeedback {
            feedback_id: "fb-comp".into(),
            incident_id: "inc-comp".into(),
            correct_hypothesis_id: Some("hyp-comp".into()),
            feedback_type: "confirmed".into(),
            operator_notes: "restart plus timeout chain was the real path".into(),
            resolved_at: "2026-05-08T10:15:00Z".into(),
            created_at: Some("2026-05-08T10:15:00Z".into()),
        })
        .expect("add feedback");
    let learning = sync_learning_artifacts(&config, &events_db, &incidents_db, &mut incidents)
        .expect("sync learning artifacts");
    assert!(learning
        .adaptive
        .learned_compositions
        .iter()
        .any(|composition| composition.cause_type == "dependency_failure"
            && composition.requires.len() >= 2
            && !composition.preferred_edge_types.is_empty()));
    assert!(learning
        .adaptive
        .learned_edge_profiles
        .iter()
        .any(|profile| profile.cause_type.as_deref() == Some("dependency_failure")));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn build_inference_graph_with_learning_applies_edge_profile_adjustment() {
    let config: TomlValue = r#"
[inference_graph]
max_events_for_graph = 50
plausibility_threshold = 0.01
max_edges_per_node = 10

[inference_graph.strategies]
dependency_propagation = false
same_service_escalation = true
resource_preceded_error = false
config_preceded_error = false
restart_preceded_disconnection = false
shared_fate = false
timeout_chain = false
"#
    .parse()
    .expect("parse config");
    let events = vec![
        event(
            "e1",
            "api",
            "warning before crash loop",
            SEVERITY_WARN,
            "app",
            "2026-05-08T10:00:00Z",
        ),
        event(
            "e2",
            "api",
            "panic and restart loop detected",
            SEVERITY_ERROR,
            "app",
            "2026-05-08T10:00:05Z",
        ),
    ];
    let baseline =
        build_inference_graph_with_learning(&config, &events, &LearningArtifacts::default());
    let baseline_edge = baseline
        .edges
        .iter()
        .find(|edge| edge.edge_type == "same_service_escalation")
        .expect("baseline edge")
        .plausibility;
    let mut learning = LearningArtifacts::default();
    learning
        .adaptive
        .learned_edge_profiles
        .push(LearnedEdgeProfile {
            profile_id: "same-service-api".into(),
            edge_type: "same_service_escalation".into(),
            source_service: Some("api".into()),
            target_service: Some("api".into()),
            cause_type: Some("service_instability".into()),
            confirmations: 4,
            false_positives: 0,
            average_plausibility: 0.95,
            average_latency_ms: 5000.0,
            created_from_feedback_id: "fb-edge".into(),
            updated_at: "2026-05-08T10:10:00Z".into(),
            manually_disabled: false,
            status_reason: None,
            review_status: default_review_status(),
            review_reason: None,
            last_reviewed_at: None,
        });
    let adapted = build_inference_graph_with_learning(&config, &events, &learning);
    let adapted_edge = adapted
        .edges
        .iter()
        .find(|edge| edge.edge_type == "same_service_escalation")
        .expect("adapted edge")
        .plausibility;
    assert!(adapted_edge > baseline_edge);
}

#[test]
fn build_adaptive_learning_history_entries_tracks_score_and_rank_movement() {
    let previous = vec![serde_json::json!({
        "hypothesis_id": "inc-1-hyp-1",
        "cause_type": "dependency_failure",
        "rank": 2,
        "total_score": 0.55,
        "score_breakdown": {
            "provenance": {
                "artifacts": [{
                    "kind": "composition",
                    "artifact_id": "composition_restart_timeout",
                    "label": "restart-timeout",
                    "impact_metric": "prior_contribution",
                    "impact_value": 0.10
                }]
            }
        }
    })];
    let next = vec![StoredHypothesis {
        hypothesis_id: "inc-1-hyp-1".into(),
        rank: Some(1),
        cause_type: "dependency_failure".into(),
        description: "learned composition".into(),
        total_score: Some(0.72),
        score_breakdown: serde_json::json!({
            "provenance": {
                "artifacts": [{
                    "kind": "composition",
                    "artifact_id": "composition_restart_timeout",
                    "label": "restart-timeout",
                    "impact_metric": "prior_contribution",
                    "impact_value": 0.17
                }]
            }
        }),
        supporting_events: vec!["e1".into()],
        contradicting_events: Vec::new(),
        affected_services: vec!["api".into()],
        suggested_checks: Vec::new(),
        confidence_label: Some("medium".into()),
        is_valid: true,
        invalidation_reasons: Vec::new(),
        created_at: "2026-05-08T12:00:00Z".into(),
        updated_at: "2026-05-08T12:00:00Z".into(),
    }];
    let entries =
        build_adaptive_learning_history_entries("inc-1", "2026-05-08T12:00:00Z", &previous, &next);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].artifact_kind, "composition");
    assert_eq!(entries[0].rank_delta, Some(1));
    assert!(entries[0]
        .score_delta
        .map(|value| (value - 0.17).abs() < 0.0001)
        .unwrap_or(false));
}
