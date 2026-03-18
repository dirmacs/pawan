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
