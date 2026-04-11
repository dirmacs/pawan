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

#[cfg(test)]
mod glob_search_tests {
    use crate::tools::native::GlobSearchTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_glob_rs_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "").unwrap();
        std::fs::write(tmp.path().join("c.txt"), "").unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"*.rs"})).await.unwrap();
        assert_eq!(r["count"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_glob_no_match() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        let tool = GlobSearchTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"*.zzz"})).await.unwrap();
        assert_eq!(r["count"].as_u64().unwrap(), 0);
    }
}

#[cfg(test)]
mod grep_native_tests {
    use crate::tools::native::GrepSearchTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_grep_with_include() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("b.py"), "def main(): pass").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"main","include":"*.rs"})).await.unwrap();
        let results = r["results"].as_str().unwrap();
        assert!(results.contains("a.rs"));
        assert!(!results.contains("b.py"));
    }

    #[tokio::test]
    async fn test_grep_count() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "aaa\nbbb\naaa\n").unwrap();
        let tool = GrepSearchTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"aaa"})).await.unwrap();
        assert!(r["count"].as_u64().unwrap() >= 1);
    }
}

#[cfg(test)]
mod write_verify_tests {
    use crate::tools::file::WriteFileTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_reports_size() {
        let tmp = TempDir::new().unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"test.txt","content":"hello world"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert_eq!(r["bytes_written"].as_u64().unwrap(), 11);
        assert!(r["size_verified"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"a/b/c/deep.txt","content":"nested"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert!(tmp.path().join("a/b/c/deep.txt").exists());
    }
}

#[cfg(test)]
mod rg_advanced_tests {
    use crate::tools::native::RipgrepTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_rg_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "Hello WORLD hello").unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"hello","case_insensitive":true})).await.unwrap();
        let matches = r["matches"].as_str().unwrap();
        assert!(matches.contains("Hello WORLD hello"));
    }

    #[tokio::test]
    async fn test_rg_fixed_strings() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "foo.bar (test)").unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"foo.bar","fixed_strings":true})).await.unwrap();
        assert!(r["match_count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn test_rg_context_lines() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "aaa\nbbb\nccc\nddd\neee").unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"ccc","context":1})).await.unwrap();
        let matches = r["matches"].as_str().unwrap();
        assert!(matches.contains("bbb"));
        assert!(matches.contains("ddd"));
    }
}

#[cfg(test)]
mod append_edge_tests {
    use crate::tools::edit::AppendFileTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_append_empty_content() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "existing\n").unwrap();
        let tool = AppendFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.txt","content":""})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_append_multiline() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "line1\n").unwrap();
        let tool = AppendFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.txt","content":"line2\nline3\nline4"})).await.unwrap();
        assert_eq!(r["lines_appended"].as_u64().unwrap(), 3);
        let c = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert!(c.contains("line1") && c.contains("line4"));
    }
}

#[cfg(test)]
mod insert_edge_tests {
    use crate::tools::edit::InsertAfterTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_insert_multiline_content() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "fn a() {}\nfn c() {}\n").unwrap();
        let tool = InsertAfterTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"fn a()","content":"fn b1() {}\nfn b2() {}"})).await.unwrap();
        assert_eq!(r["lines_inserted"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_insert_at_end_of_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "fn first() {}\nfn last() {}\n").unwrap();
        let tool = InsertAfterTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","anchor_text":"fn last()","content":"fn appended() {}"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        assert!(c.ends_with("fn appended() {}\n") || c.contains("fn appended() {}"));
    }
}

#[cfg(test)]
mod rg_invert_tests {
    use crate::tools::native::RipgrepTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_rg_invert_match() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "aaa\nbbb\nccc\n").unwrap();
        let tool = RipgrepTool::new(tmp.path().into());
        let r = tool.execute(json!({"pattern":"bbb","invert":true})).await.unwrap();
        let matches = r["matches"].as_str().unwrap();
        assert!(matches.contains("aaa"));
        assert!(matches.contains("ccc"));
        assert!(!matches.contains("bbb"));
    }
}

#[cfg(test)]
mod anchor_edge_tests {
    use crate::tools::edit::EditFileLinesTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_anchor_first_match_wins() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "fn a() {}\nfn a() {}\nfn b() {}\n").unwrap();
        let tool = EditFileLinesTool::new(tmp.path().into());
        let _r = tool.execute(json!({"path":"f.rs","anchor_text":"fn a()","anchor_count":1,"new_content":"fn replaced() {}"})).await.unwrap();
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        assert!(c.starts_with("fn replaced()"));
        assert!(c.contains("fn a() {}"));
    }
}

#[cfg(test)]
mod read_file_tests {
    use crate::tools::file::ReadFileTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_read_with_offset() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "line1\nline2\nline3\nline4\nline5\n").unwrap();
        let tool = ReadFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.txt","offset":2,"limit":2})).await.unwrap();
        let content = r["content"].as_str().unwrap();
        assert!(content.contains("line3"));
        assert!(content.contains("line4"));
        assert!(!content.contains("line1"));
        assert_eq!(r["lines_shown"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_read_default_limit() {
        let tmp = TempDir::new().unwrap();
        let big: String = (0..300).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(tmp.path().join("big.txt"), &big).unwrap();
        let tool = ReadFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"big.txt"})).await.unwrap();
        assert_eq!(r["lines_shown"].as_u64().unwrap(), 200);
        assert_eq!(r["total_lines"].as_u64().unwrap(), 300);
        // warning only fires when ALL lines shown (limit=200 means truncated, no warning)
    }

    #[tokio::test]
    async fn test_read_not_found() {
        let tmp = TempDir::new().unwrap();
        let tool = ReadFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"nope.txt"})).await;
        assert!(r.is_err());
    }
}

#[cfg(test)]
mod bash_tool_tests {
    use crate::tools::bash::BashTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_bash_echo() {
        let tmp = TempDir::new().unwrap();
        let tool = BashTool::new(tmp.path().into());
        let r = tool.execute(json!({"command":"echo hello"})).await.unwrap();
        assert!(r["stdout"].as_str().unwrap().contains("hello"));
        assert_eq!(r["exit_code"].as_i64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_bash_failure() {
        let tmp = TempDir::new().unwrap();
        let tool = BashTool::new(tmp.path().into());
        let r = tool.execute(json!({"command":"false"})).await.unwrap();
        assert_ne!(r["exit_code"].as_i64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_bash_cwd() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("marker.txt"), "found").unwrap();
        let tool = BashTool::new(tmp.path().into());
        let r = tool.execute(json!({"command":"cat marker.txt"})).await.unwrap();
        assert!(r["stdout"].as_str().unwrap().contains("found"));
    }
}

#[cfg(test)]
mod list_dir_tests {
    use crate::tools::file::ListDirectoryTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_list_dir_basic() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "").unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        let tool = ListDirectoryTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"."})).await.unwrap();
        let entries = r["entries"].as_array().unwrap();
        assert!(entries.len() >= 3);
    }

    #[tokio::test]
    async fn test_list_dir_not_found() {
        let tmp = TempDir::new().unwrap();
        let tool = ListDirectoryTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"nonexistent"})).await;
        assert!(r.is_err());
    }
}

#[cfg(test)]
mod edit_replace_all_tests {
    use crate::tools::edit::EditFileTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_replace_all() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "let x = 1;\nlet x = 2;\nlet x = 3;\n").unwrap();
        let tool = EditFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","old_string":"let x","new_string":"let y","replace_all":true})).await.unwrap();
        assert_eq!(r["replacements"].as_u64().unwrap(), 3);
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        assert!(!c.contains("let x"));
        assert_eq!(c.matches("let y").count(), 3);
    }

    #[tokio::test]
    async fn test_replace_ambiguous_fails() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "aaa\naaa\n").unwrap();
        let tool = EditFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","old_string":"aaa","new_string":"bbb"})).await;
        assert!(r.is_err()); // multiple matches without replace_all
    }
}

#[cfg(test)]
mod git_tool_tests {
    use crate::tools::git::{GitStatusTool, GitLogTool, GitDiffTool};
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_git_status_not_a_repo() {
        let tmp = TempDir::new().unwrap();
        let tool = GitStatusTool::new(tmp.path().into());
        let r = tool.execute(json!({})).await;
        // Should return error or empty status for non-git dir
        // Either is acceptable — just shouldn't panic
        let _ = r;
    }

    #[tokio::test]
    async fn test_git_status_in_repo() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(tmp.path()).output().unwrap();
        std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();
        let tool = GitStatusTool::new(tmp.path().into());
        let r = tool.execute(json!({})).await.unwrap();
        let _output = r["output"].as_str().unwrap_or("");
        // Git status should run without panicking in a valid repo
        assert!(r.is_object());
    }

    #[tokio::test]
    async fn test_git_log_empty_repo() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(tmp.path()).output().unwrap();
        let tool = GitLogTool::new(tmp.path().into());
        let r = tool.execute(json!({})).await;
        let _ = r; // empty repo log — shouldn't panic
    }

    #[tokio::test]
    async fn test_git_diff_clean() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(tmp.path()).output().unwrap();
        let tool = GitDiffTool::new(tmp.path().into());
        let r = tool.execute(json!({})).await.unwrap();
        let diff = r["diff"].as_str().unwrap_or("");
        assert!(diff.is_empty() || diff.contains("diff"));
    }
}

#[cfg(test)]
mod edit_string_match_tests {
    use crate::tools::edit::EditFileTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_edit_whitespace_matters() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "  let x = 1;\n  let y = 2;\n").unwrap();
        let tool = EditFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","old_string":"  let x = 1;","new_string":"  let x = 42;"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        assert!(c.contains("42"));
        assert!(!c.contains("= 1"));
    }

    #[tokio::test]
    async fn test_edit_empty_old_string() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "hello").unwrap();
        let tool = EditFileTool::new(tmp.path().into());
        // Empty old_string should fail (matches everything)
        let r = tool.execute(json!({"path":"f.rs","old_string":"","new_string":"x"})).await;
        // Either error or replaces nothing — both acceptable
        let _ = r;
    }

    #[tokio::test]
    async fn test_edit_newline_in_match() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "fn main() {\n    hello();\n}\n").unwrap();
        let tool = EditFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","old_string":"fn main() {\n    hello();\n}","new_string":"fn main() {\n    world();\n}"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("f.rs")).unwrap();
        assert!(c.contains("world()"));
    }
}

#[cfg(test)]
mod git_write_tests {
    use crate::tools::git::{GitAddTool, GitCommitTool};
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_git_add_in_repo() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.email","test@test.com"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.name","test"]).current_dir(tmp.path()).output().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "hello").unwrap();
        let tool = GitAddTool::new(tmp.path().into());
        let r = tool.execute(json!({"files":["f.txt"]})).await.unwrap();
        assert!(r.is_object());
    }

    #[tokio::test]
    async fn test_git_commit_after_add() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.email","test@test.com"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.name","test"]).current_dir(tmp.path()).output().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "hello").unwrap();
        std::process::Command::new("git").args(["add","f.txt"]).current_dir(tmp.path()).output().unwrap();
        let tool = GitCommitTool::new(tmp.path().into());
        let r = tool.execute(json!({"message":"test commit"})).await.unwrap();
        assert!(r.is_object());
    }
}

#[cfg(test)]
mod git_extended_tests {
    use crate::tools::git::{GitBlameTool, GitBranchTool};
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    fn init_repo(tmp: &TempDir) {
        std::process::Command::new("git").args(["init"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.email","t@t.com"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.name","t"]).current_dir(tmp.path()).output().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "hello").unwrap();
        std::process::Command::new("git").args(["add","."]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["commit","-m","init"]).current_dir(tmp.path()).output().unwrap();
    }

    #[tokio::test]
    async fn test_git_blame() {
        let tmp = TempDir::new().unwrap();
        init_repo(&tmp);
        let tool = GitBlameTool::new(tmp.path().into());
        let r = tool.execute(json!({"file":"f.txt"})).await.unwrap();
        assert!(r.is_object());
    }

    #[tokio::test]
    async fn test_git_branch_list() {
        let tmp = TempDir::new().unwrap();
        init_repo(&tmp);
        let tool = GitBranchTool::new(tmp.path().into());
        let r = tool.execute(json!({})).await.unwrap();
        assert!(r.is_object());
    }
}

#[cfg(test)]
mod git_checkout_stash_tests {
    use crate::tools::git::{GitCheckoutTool, GitStashTool};
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    fn init_repo(tmp: &TempDir) {
        std::process::Command::new("git").args(["init"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.email","t@t.com"]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["config","user.name","t"]).current_dir(tmp.path()).output().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "v1").unwrap();
        std::process::Command::new("git").args(["add","."]).current_dir(tmp.path()).output().unwrap();
        std::process::Command::new("git").args(["commit","-m","init"]).current_dir(tmp.path()).output().unwrap();
    }

    #[tokio::test]
    async fn test_git_checkout_new_branch() {
        let tmp = TempDir::new().unwrap();
        init_repo(&tmp);
        let tool = GitCheckoutTool::new(tmp.path().into());
        let r = tool.execute(json!({"branch":"test-branch","create":true})).await;
        let _ = r; // shouldn't panic
    }

    #[tokio::test]
    async fn test_git_stash_empty() {
        let tmp = TempDir::new().unwrap();
        init_repo(&tmp);
        let tool = GitStashTool::new(tmp.path().into());
        let r = tool.execute(json!({})).await;
        let _ = r; // stashing clean tree — shouldn't panic
    }
}

#[cfg(test)]
mod agent_tool_tests {
    use crate::tools::agent::SpawnAgentTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_spawn_agent_missing_prompt() {
        let tmp = TempDir::new().unwrap();
        let tool = SpawnAgentTool::new(tmp.path().into());
        let r = tool.execute(json!({})).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_spawn_agent_schema() {
        let tool = SpawnAgentTool::new(std::path::PathBuf::from("/tmp"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["prompt"].is_object());
    }
}

#[cfg(test)]
mod tool_registry_tests {
    use crate::tools::ToolRegistry;

    #[test]
    fn test_registry_no_duplicate_names() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = ToolRegistry::with_defaults(tmp.path().into());
        let defs = registry.get_definitions();
        let mut names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "Duplicate tool names found!");
    }

    #[test]
    fn test_registry_all_have_descriptions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = ToolRegistry::with_defaults(tmp.path().into());
        for def in registry.get_definitions() {
            assert!(!def.description.is_empty(), "Tool {} has no description", def.name);
            assert!(def.description.len() > 10, "Tool {} description too short", def.name);
        }
    }
}

#[cfg(test)]
mod milestone_tests {
    #[test]
    fn test_minimum_tool_count() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = crate::tools::ToolRegistry::with_defaults(tmp.path().into());
        let defs = registry.get_all_definitions();
        assert!(defs.len() >= 28, "Expected at least 28 tools, got {}", defs.len());
    }

    #[test]
    fn test_all_tool_categories_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = crate::tools::ToolRegistry::with_defaults(tmp.path().into());
        let names: Vec<String> = registry.get_all_definitions().iter().map(|d| d.name.clone()).collect();
        // File tools
        assert!(names.contains(&"read_file".into()));
        assert!(names.contains(&"write_file".into()));
        assert!(names.contains(&"append_file".into()));
        // Edit tools
        assert!(names.contains(&"edit_file".into()));
        assert!(names.contains(&"edit_file_lines".into()));
        assert!(names.contains(&"insert_after".into()));
        // Native tools
        assert!(names.contains(&"rg".into()));
        assert!(names.contains(&"fd".into()));
        assert!(names.contains(&"sd".into()));
        assert!(names.contains(&"z".into()));
        assert!(names.contains(&"mise".into()));
        // Git tools
        assert!(names.contains(&"git_status".into()));
        assert!(names.contains(&"git_commit".into()));
        // Agent tools
        assert!(names.contains(&"spawn_agent".into()));
        // Bash
        assert!(names.contains(&"bash".into()));
    }
}

#[cfg(test)]
mod diff_tests {
    use crate::tools::edit::EditFileTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_edit_returns_diff() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "let old = 1;\n").unwrap();
        let tool = EditFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","old_string":"let old = 1;","new_string":"let new = 2;"})).await.unwrap();
        let diff = r["diff"].as_str().unwrap();
        assert!(diff.contains("-let old = 1;"));
        assert!(diff.contains("+let new = 2;"));
    }

    #[tokio::test]
    async fn test_edit_lines_returns_diff() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.rs"), "aaa\nbbb\nccc\n").unwrap();
        let tool = crate::tools::edit::EditFileLinesTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"f.rs","start_line":2,"end_line":2,"new_content":"XXX"})).await.unwrap();
        let diff = r["diff"].as_str().unwrap();
        assert!(diff.contains("-bbb"));
        assert!(diff.contains("+XXX"));
    }
}

#[cfg(test)]
mod write_file_edge_tests {
    use crate::tools::file::WriteFileTool;
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_overwrites() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f.txt"), "old content").unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        tool.execute(json!({"path":"f.txt","content":"new content"})).await.unwrap();
        let c = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert_eq!(c, "new content");
        assert!(!c.contains("old"));
    }

    #[tokio::test]
    async fn test_write_empty_file() {
        let tmp = TempDir::new().unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"empty.txt","content":""})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert_eq!(r["bytes_written"].as_u64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_write_unicode() {
        let tmp = TempDir::new().unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path":"uni.txt","content":"hello 世界 🦀"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert!(r["size_verified"].as_bool().unwrap());
        let c = std::fs::read_to_string(tmp.path().join("uni.txt")).unwrap();
        assert!(c.contains("🦀"));
    }
}

#[cfg(test)]
mod config_tests {
    use crate::config::PawanConfig;

    #[test]
    fn test_default_config() {
        let config = PawanConfig::default();
        assert!(!config.model.is_empty());
        assert!(config.temperature > 0.0);
        assert!(config.max_tokens > 0);
        assert!(config.max_tool_iterations > 0);
        assert!(config.max_context_tokens > 0);
        assert!(config.cloud.is_none());
    }

    #[test]
    fn test_system_prompt() {
        let config = PawanConfig::default();
        let prompt = config.get_system_prompt();
        assert!(prompt.contains("Pawan"));
        assert!(prompt.contains("tools"));
        assert!(prompt.len() > 100);
    }

    #[test]
    fn test_thinking_mode_supported_models() {
        let mut config = PawanConfig::default();
        config.reasoning_mode = true;

        // Non-thinking models (no chat_template_kwargs or reasoning_effort)
        config.model = "stepfun-ai/step-3.5-flash".into();
        assert!(!config.use_thinking_mode());
        config.model = "minimaxai/minimax-m2.5".into();
        assert!(!config.use_thinking_mode());

        // Thinking-capable models
        config.model = "deepseek-ai/deepseek-v3".into();
        assert!(config.use_thinking_mode());
        config.model = "google/gemma-4-31b-it".into();
        assert!(config.use_thinking_mode());
        config.model = "z-ai/glm4.7".into();
        assert!(config.use_thinking_mode());
        config.model = "qwen/qwen3.5-122b-a10b".into();
        assert!(config.use_thinking_mode());
        config.model = "mistralai/mistral-small-4-119b-2603".into();
        assert!(config.use_thinking_mode());

        // reasoning_mode=false disables it for all
        config.reasoning_mode = false;
        config.model = "deepseek-ai/deepseek-v3".into();
        assert!(!config.use_thinking_mode());
        config.model = "mistralai/mistral-small-4-119b-2603".into();
        assert!(!config.use_thinking_mode());
    }
}

#[cfg(test)]
mod env_override_tests {
    use crate::config::PawanConfig;

    #[test]
    fn test_env_override_model() {
        std::env::set_var("PAWAN_MODEL", "test-model-override");
        let mut config = PawanConfig::default();
        config.apply_env_overrides();
        assert_eq!(config.model, "test-model-override");
        std::env::remove_var("PAWAN_MODEL");
    }

    #[test]
    fn test_env_override_temperature() {
        std::env::set_var("PAWAN_TEMPERATURE", "0.42");
        let mut config = PawanConfig::default();
        config.apply_env_overrides();
        assert!((config.temperature - 0.42).abs() < 0.01);
        std::env::remove_var("PAWAN_TEMPERATURE");
    }

    #[test]
    fn test_env_override_fallback_models() {
        std::env::set_var("PAWAN_FALLBACK_MODELS", "model-a, model-b, model-c");
        let mut config = PawanConfig::default();
        config.apply_env_overrides();
        assert_eq!(config.fallback_models.len(), 3);
        assert_eq!(config.fallback_models[0], "model-a");
        std::env::remove_var("PAWAN_FALLBACK_MODELS");
    }
}

#[cfg(test)]
mod healing_config_tests {
    use crate::config::PawanConfig;

    #[test]
    fn test_healing_defaults() {
        let config = PawanConfig::default();
        assert!(config.healing.fix_errors);
        assert!(config.healing.fix_warnings);
        assert!(config.healing.fix_tests);
        assert!(!config.healing.auto_commit);
        // fix_security defaults to OFF — cargo audit needs the binary
        // installed and can hit the network for the advisory database.
        assert!(!config.healing.fix_security);
    }
}

#[cfg(test)]
mod cloud_config_tests {
    use crate::config::{PawanConfig, CloudConfig, LlmProvider};

    #[test]
    fn test_cloud_config_none_by_default() {
        let config = PawanConfig::default();
        assert!(config.cloud.is_none());
    }

    #[test]
    fn test_cloud_config_creation() {
        let cloud = CloudConfig {
            provider: LlmProvider::Nvidia,
            model: "test-model".into(),
            fallback_models: vec!["fb1".into(), "fb2".into()],
        };
        assert_eq!(cloud.model, "test-model");
        assert_eq!(cloud.fallback_models.len(), 2);
    }
}

#[cfg(test)]
mod permission_tests {
    use crate::config::{PawanConfig, ToolPermission};

    #[test]
    fn test_no_permissions_by_default() {
        let config = PawanConfig::default();
        assert!(config.permissions.is_empty());
    }

    #[test]
    fn test_permission_deny() {
        let mut config = PawanConfig::default();
        config.permissions.insert("bash".into(), ToolPermission::Deny);
        assert_eq!(config.permissions.get("bash"), Some(&ToolPermission::Deny));
    }

    #[test]
    fn test_permission_prompt() {
        let mut config = PawanConfig::default();
        config.permissions.insert("bash".into(), ToolPermission::Prompt);
        assert_eq!(config.permissions.get("bash"), Some(&ToolPermission::Prompt));
    }

    #[test]
    fn test_resolve_explicit_override() {
        use std::collections::HashMap;
        let mut perms = HashMap::new();
        perms.insert("bash".into(), ToolPermission::Deny);
        assert_eq!(ToolPermission::resolve("bash", &perms), ToolPermission::Deny);

        perms.insert("bash".into(), ToolPermission::Prompt);
        assert_eq!(ToolPermission::resolve("bash", &perms), ToolPermission::Prompt);
    }

    #[test]
    fn test_resolve_default_allow() {
        use std::collections::HashMap;
        let perms = HashMap::new();
        // All tools default to Allow when not configured
        assert_eq!(ToolPermission::resolve("bash", &perms), ToolPermission::Allow);
        assert_eq!(ToolPermission::resolve("read_file", &perms), ToolPermission::Allow);
        assert_eq!(ToolPermission::resolve("glob_search", &perms), ToolPermission::Allow);
    }

    #[test]
    fn test_permission_toml_parsing() {
        let toml = r#"
model = "test"
[permissions]
bash = "deny"
write_file = "prompt"
read_file = "allow"
"#;
        let config: PawanConfig = toml::from_str(toml).expect("should parse");
        assert_eq!(config.permissions.get("bash"), Some(&ToolPermission::Deny));
        assert_eq!(config.permissions.get("write_file"), Some(&ToolPermission::Prompt));
        assert_eq!(config.permissions.get("read_file"), Some(&ToolPermission::Allow));
    }
}

#[cfg(test)]
mod file_tool_tests {
    use crate::tools::file::{ReadFileTool, WriteFileTool, ListDirectoryTool};
    use crate::tools::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_empty_content() {
        let tmp = TempDir::new().unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path": "empty.txt", "content": ""})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(tmp.path().join("empty.txt")).unwrap(), "");
    }

    #[tokio::test]
    async fn test_read_offset_beyond_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("short.txt"), "line1\nline2\n").unwrap();
        let tool = ReadFileTool::new(tmp.path().into());
        // Offset beyond file length should return empty or gracefully handle
        let r = tool.execute(json!({"path": "short.txt", "offset": 999, "limit": 10})).await.unwrap();
        // Should not panic — either empty content or an error, but not a crash
        assert!(r.get("content").is_some() || r.get("error").is_some());
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path": "nested/deep/file.txt", "content": "hello"})).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(tmp.path().join("nested/deep/file.txt")).unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_list_directory_empty() {
        let tmp = TempDir::new().unwrap();
        let tool = ListDirectoryTool::new(tmp.path().into());
        let r = tool.execute(json!({"path": "."})).await.unwrap();
        // Empty directory — should succeed without panicking
        assert!(r.get("entries").is_some() || r.get("error").is_some());
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let tool = ReadFileTool::new(tmp.path().into());
        let r = tool.execute(json!({"path": "does_not_exist.rs"})).await;
        assert!(r.is_err());
    }
}

#[cfg(test)]
mod file_write_safety_tests {
    use crate::tools::file::validate_file_write;
    use std::path::Path;

    #[test]
    fn test_blocks_git_directory() {
        assert!(validate_file_write(Path::new(".git/config")).is_err());
        assert!(validate_file_write(Path::new(".git/hooks/pre-commit")).is_err());
        assert!(validate_file_write(Path::new("src/.git/HEAD")).is_err());
    }

    #[test]
    fn test_blocks_credential_files() {
        assert!(validate_file_write(Path::new(".env")).is_err());
        assert!(validate_file_write(Path::new(".env.local")).is_err());
        assert!(validate_file_write(Path::new(".env.production")).is_err());
        assert!(validate_file_write(Path::new("id_rsa")).is_err());
        assert!(validate_file_write(Path::new("id_ed25519")).is_err());
        assert!(validate_file_write(Path::new("credentials.json")).is_err());
        assert!(validate_file_write(Path::new(".npmrc")).is_err());
    }

    #[test]
    fn test_blocks_system_paths() {
        assert!(validate_file_write(Path::new("/etc/passwd")).is_err());
        assert!(validate_file_write(Path::new("/usr/bin/something")).is_err());
        assert!(validate_file_write(Path::new("/bin/sh")).is_err());
        assert!(validate_file_write(Path::new("/boot/grub")).is_err());
    }

    #[test]
    fn test_allows_normal_files() {
        assert!(validate_file_write(Path::new("src/main.rs")).is_ok());
        assert!(validate_file_write(Path::new("README.md")).is_ok());
        assert!(validate_file_write(Path::new("Cargo.toml")).is_ok());
        assert!(validate_file_write(Path::new("tests/integration.rs")).is_ok());
        assert!(validate_file_write(Path::new("docs/guide.md")).is_ok());
    }

    #[test]
    fn test_allows_lock_files_with_warning() {
        // Lock files are allowed (just warned about)
        assert!(validate_file_write(Path::new("Cargo.lock")).is_ok());
        assert!(validate_file_write(Path::new("package-lock.json")).is_ok());
    }

    #[test]
    fn test_allows_dotfiles_not_in_blocklist() {
        assert!(validate_file_write(Path::new(".gitignore")).is_ok());
        assert!(validate_file_write(Path::new(".cargo/config.toml")).is_ok());
        assert!(validate_file_write(Path::new(".github/workflows/ci.yml")).is_ok());
    }

    #[test]
    fn test_blocks_nested_env_file() {
        // .env inside a subdirectory should still be blocked
        assert!(validate_file_write(Path::new("config/.env")).is_err());
        assert!(validate_file_write(Path::new("deploy/.env.production")).is_err());
    }

    #[tokio::test]
    async fn test_write_tool_blocks_env_file() {
        use crate::tools::file::WriteFileTool;
        use crate::tools::Tool;
        use serde_json::json;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let tool = WriteFileTool::new(tmp.path().into());
        let result = tool.execute(json!({"path": ".env", "content": "SECRET=bad"})).await;
        assert!(result.is_err(), "Writing .env should be blocked");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("blocked") || err.contains("credential"), "Error: {}", err);
    }
}
