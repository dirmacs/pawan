#[cfg(test)]
mod anchor_tests {
    use crate::tools::edit::{EditFileLinesTool, InsertAfterTool, AppendFileTool};
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_anchor_mode() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "line_a\nline_b\nline_c\n").unwrap();
        let tool = EditFileLinesTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"line_b","anchor_count":1,"new_content":"replaced"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(tmp.path().join("f.rs")).unwrap(), "line_a\nreplaced\nline_c\n");
    }

    #[tokio::test]
    async fn test_anchor_multi_line() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "a\nb\nc\nd\ne\n").unwrap();
        let tool = EditFileLinesTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"b","anchor_count":3,"new_content":"X"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(tmp.path().join("f.rs")).unwrap(), "a\nX\ne\n");
    }

    #[tokio::test]
    async fn test_anchor_not_found() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "hello\nworld\n").unwrap();
        let tool = EditFileLinesTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"nope","new_content":"x"})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_insert_after() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "a\nb\nc\n").unwrap();
        let tool = InsertAfterTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"b","content":"inserted"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(tmp.path().join("f.rs")).unwrap(), "a\nb\ninserted\nc\n");
    }

    #[tokio::test]
    async fn test_insert_not_found() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "a\nb\n").unwrap();
        let tool = InsertAfterTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"z","content":"x"})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_append_new() {
        let tmp = TempDir::new().unwrap();
        let tool = AppendFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"new.txt","content":"hello"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert!(std::fs::read_to_string(tmp.path().join("new.txt")).unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn test_append_existing() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "old\n").unwrap();
        let tool = AppendFileTool::new(tmp.path().into());
        tool.execute(json!({"path":"f.txt","content":"new"})).await.unwrap();
        let c = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert!(c.contains("old") && c.contains("new"));
    }

    #[tokio::test]
    async fn test_replaced_content_in_response() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "aa\nbb\ncc\n").unwrap();
        let tool = EditFileLinesTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","start_line":2,"end_line":2,"new_content":"XX"})).await.unwrap();
        assert!(r["replaced_content"].as_str().unwrap().contains("bb"));
    }

    #[tokio::test]
    async fn test_anchor_with_special_chars() {
        let tmp = TempDir::new().unwrap();
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        std::fs::write(tmp.path().join("f.rs"), content).unwrap();
        let tool = EditFileLinesTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"println","anchor_count":1,"new_content":"    println!(\"world\");"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        assert!(c.contains("world"));
        assert!(!c.contains("hello"));
    }

    #[tokio::test]
    async fn test_insert_after_block_aware() {
        let tmp = TempDir::new().unwrap();
        let content = "fn foo() {\n    println!(\"hello\");\n}\nfn bar() {}";
        std::fs::write(tmp.path().join("f.rs"), content).unwrap();
        let tool = InsertAfterTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"fn foo()","content":"fn inserted() {}"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        // Should insert AFTER foo's closing }, not inside foo's body
        assert!(c.contains("}\nfn inserted() {}\nfn bar()"), "Got: {}", c);
    }

    #[tokio::test]
    async fn test_insert_after_no_block() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "use std::io;\nuse std::fs;\n").unwrap();
        let tool = InsertAfterTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"use std::io","content":"use std::path;"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        // No block — should insert right after the anchor line
        assert_eq!(c, "use std::io;\nuse std::path;\nuse std::fs;\n");
    }
}

#[cfg(test)]
mod native_tests {
    use crate::tools::native::SdTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_sd_replace() {
        if which::which("sd").is_err() { return; }
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "hello world hello").unwrap();
        let tool = SdTool::new(tmp.path().into());
        let r = tool.execute(json!({"find":"hello","replace":"hi","path":"f.txt","fixed_strings":true})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert_eq!(c, "hi world hi");
    }

    #[tokio::test]
    async fn test_sd_missing_replace() {
        let tmp = TempDir::new().unwrap();
        let tool = SdTool::new(tmp.path().into());
        let r = tool.execute(json!({"find":"x","path":"f.txt"})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_sd_missing_path() {
        let tmp = TempDir::new().unwrap();
        let tool = SdTool::new(tmp.path().into());
        let r = tool.execute(json!({"find":"x","replace":"y"})).await;
        assert!(r.is_err());
    }
}

#[cfg(test)]
mod rg_tests {
    use crate::tools::native::RipgrepTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_rg_basic() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn main() {\n    println!(\"hello\");\n}").unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"println"})).await.unwrap();
        assert!(r["match_count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn test_rg_no_match() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello world").unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"zzzzz"})).await.unwrap();
        assert_eq!(r["match_count"].as_u64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_rg_type_filter() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn test()").unwrap();
        std::fs::write(tmp.path().join("b.py"), "def test()").unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"test","type_filter":"rust"})).await.unwrap();
        let matches = r["matches"].as_str().unwrap();
        assert!(matches.contains("a.rs"));
        assert!(!matches.contains("b.py"));
    }
}

#[cfg(test)]
mod fd_tests {
    use crate::tools::native::FdTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_fd_find_by_name() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("foo.rs"), "").unwrap();
        std::fs::write(tmp.path().join("bar.rs"), "").unwrap();
        std::fs::write(tmp.path().join("baz.txt"), "").unwrap();
        let tool = FdTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"foo"})).await.unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].as_str().unwrap().contains("foo.rs"));
    }

    #[tokio::test]
    async fn test_fd_extension_filter() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "").unwrap();
        std::fs::write(tmp.path().join("c.txt"), "").unwrap();
        let tool = FdTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":".", "extension":"rs"})).await.unwrap();
        assert_eq!(r["count"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_fd_no_results() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        let tool = FdTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"zzzzz"})).await.unwrap();
        assert_eq!(r["count"].as_u64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_fd_max_depth() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
        std::fs::write(tmp.path().join("top.rs"), "").unwrap();
        std::fs::write(tmp.path().join("a/mid.rs"), "").unwrap();
        std::fs::write(tmp.path().join("a/b/c/deep.rs"), "").unwrap();
        let tool = FdTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":".rs","max_depth":1})).await.unwrap();
        let files = r["files"].as_array().unwrap();
        assert!(files.len() <= 1);
    }
}

#[cfg(test)]
mod zoxide_tests {
    use crate::tools::native::ZoxideTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_z_add_and_query() {
        if which::which("zoxide").is_err() { return; }
        let tmp = TempDir::new().unwrap();
        let tool = ZoxideTool::new(tmp.path().into());
        let r = tool.execute(json!({"action":"add","path":"/tmp"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_z_list() {
        if which::which("zoxide").is_err() { return; }
        let tmp = TempDir::new().unwrap();
        let tool = ZoxideTool::new(tmp.path().into());
        let r = tool.execute(json!({"action":"list"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_z_bad_action() {
        let tmp = TempDir::new().unwrap();
        let tool = ZoxideTool::new(tmp.path().into());
        let r = tool.execute(json!({"action":"invalid"})).await;
        assert!(r.is_err());
    }
}

#[cfg(test)]
mod erd_tests {
    use crate::tools::native::ErdTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_tree_basic() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        let tool = ErdTool::new(tmp.path().into());
        let r = tool.execute(json!({"depth":2})).await.unwrap();
        let tree = r["tree"].as_str().unwrap();
        assert!(tree.contains("main.rs") || tree.contains("src") || tree.contains("Cargo"));
    }

    #[tokio::test]
    async fn test_tree_depth_limit() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b/c/d")).unwrap();
        std::fs::write(tmp.path().join("a/b/c/d/deep.txt"), "").unwrap();
        let tool = ErdTool::new(tmp.path().into());
        let r = tool.execute(json!({"depth":1})).await.unwrap();
        let tree = r["tree"].as_str().unwrap();
        assert!(!tree.contains("deep.txt"));
    }
}

#[cfg(test)]
mod mise_tests {
    use crate::tools::native::MiseTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_mise_bad_action() {
        let tmp = TempDir::new().unwrap();
        let tool = MiseTool::new(tmp.path().into());
        let r = tool.execute(json!({"action":"invalid"})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_mise_install_missing_tool() {
        let tmp = TempDir::new().unwrap();
        let tool = MiseTool::new(tmp.path().into());
        let r = tool.execute(json!({"action":"install"})).await;
        assert!(r.is_err()); // missing tool name
    }
}
