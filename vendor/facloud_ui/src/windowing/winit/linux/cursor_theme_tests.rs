use super::CursorThemeCrawler;

enum Stub<'a> {
    FileWithContent(&'a str, &'a str),
}

struct TestDirs {
    root: std::path::PathBuf,
}

impl TestDirs {
    fn tests(&self) -> &std::path::Path {
        &self.root
    }
}

struct Sandbox {
    root: std::path::PathBuf,
}

impl Sandbox {
    fn mkdir(&mut self, rel: &str) {
        std::fs::create_dir_all(self.root.join(rel)).unwrap();
    }

    fn with_files(&mut self, files: Vec<Stub<'_>>) {
        for Stub::FileWithContent(rel, content) in files {
            let path = self.root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
    }
}

struct VirtualFS;

impl VirtualFS {
    fn test<F: FnOnce(&TestDirs, Sandbox)>(name: &str, f: F) {
        let root = std::env::temp_dir().join(format!(
            "facloud_ui_cursor_theme_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dirs = TestDirs { root: root.clone() };
        let sandbox = Sandbox { root: root.clone() };
        f(&dirs, sandbox);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn test_no_themes_found() {
    VirtualFS::test("test_no_themes_found", |dirs, mut sandbox| {
        sandbox.mkdir("icons");
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(crawler.determine_cursor_theme(), None);
    });
}

#[test]
fn test_default_theme_found() {
    VirtualFS::test("test_default_theme_found", |dirs, mut sandbox| {
        sandbox.mkdir("icons/default/cursors");
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}

#[test]
fn test_known_theme_found() {
    VirtualFS::test("test_known_theme_found", |dirs, mut sandbox| {
        sandbox.mkdir("icons/Yaru/cursors");
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(crawler.determine_cursor_theme(), Some("Yaru".to_string()));
    });
}

#[test]
fn test_default_theme_found_via_index() {
    VirtualFS::test("test_default_theme_found_via_index", |dirs, mut sandbox| {
        sandbox.mkdir("icons/Darmok/cursors");
        sandbox.mkdir("icons/default");
        sandbox.with_files(vec![Stub::FileWithContent(
            "icons/default/index.theme",
            r#"
            [Icon Theme]
            Inherits=Darmok
            "#,
        )]);
        let crawler: CursorThemeCrawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}

#[test]
fn test_default_theme_is_prioritized_over_known_theme() {
    VirtualFS::test(
        "test_default_theme_is_prioritized_over_known_theme",
        |dirs, mut sandbox| {
            sandbox.mkdir("icons/Darmok/cursors");
            sandbox.mkdir("icons/Yaru/cursors");
            sandbox.mkdir("icons/default");
            sandbox.with_files(vec![Stub::FileWithContent(
                "icons/default/index.theme",
                r#"
            [Icon Theme]
            Inherits=Darmok
            "#,
            )]);
            let crawler = CursorThemeCrawler {
                directories: vec![dirs.tests().join("icons")],
            };

            assert_eq!(
                crawler.determine_cursor_theme(),
                Some("default".to_string())
            );
        },
    );
}

#[test]
fn test_multiple_directories() {
    VirtualFS::test("test_multiple_directories", |dirs, mut sandbox| {
        sandbox.mkdir("icons2/Darmok/cursors");
        sandbox.mkdir("icons/default");
        sandbox.with_files(vec![Stub::FileWithContent(
            "icons/default/index.theme",
            r#"
            [Icon Theme]
            Inherits=Darmok
            "#,
        )]);
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons"), dirs.tests().join("icons2")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}
#[test]
fn test_resolution_order() {
    VirtualFS::test("test_resolution_order", |dirs, mut sandbox| {
        sandbox.mkdir("icons2/Darmok/cursors");
        sandbox.mkdir("icons/default");
        sandbox.mkdir("icons2/default");
        sandbox.with_files(vec![
            Stub::FileWithContent(
                "icons/default/index.theme",
                r#"
                [Icon Theme]
                Inherits=Jalad
                "#,
            ),
            Stub::FileWithContent(
                "icons2/default/index.theme",
                r#"
                [Icon Theme]
                Inherits=Darmok
                "#,
            ),
        ]);

        // Case 1: we find the index file in icons first.
        // The index file points to a non-existent theme Jalad,
        // so we return None
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons"), dirs.tests().join("icons2")],
        };

        assert_eq!(crawler.determine_cursor_theme(), None);

        // Case 2: we find the index file in icons first.
        // The index file points to the valid theme Darmok,
        // so we return Some("default")
        let crawler = CursorThemeCrawler {
            directories: vec![dirs.tests().join("icons2"), dirs.tests().join("icons")],
        };

        assert_eq!(
            crawler.determine_cursor_theme(),
            Some("default".to_string())
        );
    });
}
