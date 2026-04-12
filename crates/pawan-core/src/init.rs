//! Project skeleton templates for `pawan init`.
//!
//! Generates scaffolding files for new Rust projects that pass `cargo check` immediately.

use std::path::Path;

/// A project skeleton: a named collection of (relative path, content) pairs.
pub struct ProjectSkeleton {
    pub name: String,
    pub files: Vec<(String, String)>,
}

impl ProjectSkeleton {
    /// Write all skeleton files under `root`, creating intermediate directories as needed.
    pub fn write_to(&self, root: &Path) -> std::io::Result<()> {
        for (rel_path, content) in &self.files {
            let full = root.join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full, content)?;
        }
        Ok(())
    }
}

/// Generate a simple Rust binary project skeleton.
pub fn rust_binary_skeleton(name: &str) -> ProjectSkeleton {
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"
"#
    );
    let main_rs = "fn main() {\n    println!(\"Hello, world!\");\n}\n".to_string();

    ProjectSkeleton {
        name: name.to_string(),
        files: vec![
            ("Cargo.toml".into(), cargo_toml),
            ("src/main.rs".into(), main_rs),
        ],
    }
}

/// Generate a Rust library project skeleton.
pub fn rust_library_skeleton(name: &str) -> ProjectSkeleton {
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"
"#
    );
    let lib_rs = format!(
        "/// Add two numbers.\npub fn add(a: i32, b: i32) -> i32 {{\n    a + b\n}}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n\n    #[test]\n    fn it_works() {{\n        assert_eq!(add(2, 2), 4);\n    }}\n}}\n"
    );

    ProjectSkeleton {
        name: name.to_string(),
        files: vec![
            ("Cargo.toml".into(), cargo_toml),
            ("src/lib.rs".into(), lib_rs),
        ],
    }
}

/// Generate a Pawan agent project skeleton (binary with pawan dep + pawan.toml config).
pub fn pawan_agent_skeleton(name: &str) -> ProjectSkeleton {
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[dependencies]
pawan = {{ git = "https://github.com/dirmacs/pawan.git" }}
"#
    );
    let main_rs = format!(
        r#"fn main() {{
    println!("Pawan agent: {name}");
}}
"#
    );
    let pawan_toml = format!(
        r#"[agent]
name = "{name}"
model = "qwen/qwen3.5-122b-a10b"

[provider]
name = "nvidia"
api_url = "https://integrate.api.nvidia.com/v1"
"#
    );

    ProjectSkeleton {
        name: name.to_string(),
        files: vec![
            ("Cargo.toml".into(), cargo_toml),
            ("src/main.rs".into(), main_rs),
            ("pawan.toml".into(), pawan_toml),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn binary_skeleton_files() {
        let sk = rust_binary_skeleton("myapp");
        assert_eq!(sk.name, "myapp");
        let paths: Vec<&str> = sk.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["Cargo.toml", "src/main.rs"]);
    }

    #[test]
    fn library_skeleton_files() {
        let sk = rust_library_skeleton("mylib");
        assert_eq!(sk.name, "mylib");
        let paths: Vec<&str> = sk.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["Cargo.toml", "src/lib.rs"]);
    }

    #[test]
    fn agent_skeleton_files() {
        let sk = pawan_agent_skeleton("myagent");
        assert_eq!(sk.name, "myagent");
        let paths: Vec<&str> = sk.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["Cargo.toml", "src/main.rs", "pawan.toml"]);
    }

    #[test]
    fn write_to_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let sk = rust_binary_skeleton("testproj");
        sk.write_to(dir.path()).unwrap();

        let cargo = fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"testproj\""));
        assert!(cargo.contains("edition = \"2021\""));
        assert!(cargo.contains("rust-version = \"1.75\""));

        let main = fs::read_to_string(dir.path().join("src/main.rs")).unwrap();
        assert!(main.contains("fn main()"));
    }

    #[test]
    fn cargo_toml_is_valid_toml() {
        for sk in [
            rust_binary_skeleton("a"),
            rust_library_skeleton("b"),
            pawan_agent_skeleton("c"),
        ] {
            let cargo_content = &sk.files.iter().find(|(p, _)| p == "Cargo.toml").unwrap().1;
            let parsed: Result<toml::Value, _> = toml::from_str(cargo_content);
            assert!(parsed.is_ok(), "Invalid TOML in {} skeleton", sk.name);
        }
    }

    #[test]
    fn pawan_agent_skeleton_includes_pawan_toml_in_file_list() {
        let sk = pawan_agent_skeleton("demo");
        let has_pawan_toml = sk.files.iter().any(|(p, _)| p == "pawan.toml");
        assert!(has_pawan_toml, "pawan_agent_skeleton must include pawan.toml");
    }

    #[test]
    fn write_to_creates_nested_directories() {
        let dir = tempfile::tempdir().unwrap();
        let sk = rust_binary_skeleton("nested");
        sk.write_to(dir.path()).unwrap();
        // src/ directory must have been created for src/main.rs
        assert!(dir.path().join("src").is_dir(), "src/ directory not created");
        assert!(dir.path().join("src/main.rs").is_file());
    }

    #[test]
    fn skeleton_names_set_correctly() {
        assert_eq!(rust_binary_skeleton("alpha").name, "alpha");
        assert_eq!(rust_library_skeleton("beta").name, "beta");
        assert_eq!(pawan_agent_skeleton("gamma").name, "gamma");
    }

    #[test]
    fn generated_cargo_toml_contains_edition_and_rust_version() {
        for (sk, expected_name) in [
            (rust_binary_skeleton("x"), "x"),
            (rust_library_skeleton("y"), "y"),
            (pawan_agent_skeleton("z"), "z"),
        ] {
            let cargo = &sk.files.iter().find(|(p, _)| p == "Cargo.toml").unwrap().1;
            assert!(cargo.contains("edition = \"2021\""), "{expected_name} missing edition");
            assert!(cargo.contains("rust-version = \"1.75\""), "{expected_name} missing rust-version");
        }
    }

    #[test]
    fn agent_skeleton_has_pawan_toml() {
        let dir = tempfile::tempdir().unwrap();
        let sk = pawan_agent_skeleton("agent1");
        sk.write_to(dir.path()).unwrap();

        let pawan_cfg = fs::read_to_string(dir.path().join("pawan.toml")).unwrap();
        assert!(pawan_cfg.contains("name = \"agent1\""));
        assert!(pawan_cfg.contains("model ="));
    }
}
