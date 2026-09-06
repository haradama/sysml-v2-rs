//! The files a project has, found by walking a directory.

use std::path::PathBuf;

/// A fresh directory under the system's temporary one, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("sysml-model-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn model_files_are_found_below_a_directory_in_a_stable_order() {
    let scratch = Scratch::new("order");
    let dir = &scratch.0;
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("b.sysml"), "package B;").unwrap();
    std::fs::write(dir.join("sub").join("a.kerml"), "package A;").unwrap();
    std::fs::write(dir.join("notes.txt"), "not a model").unwrap();
    let names: Vec<String> = sysml_semantics::model_files(dir)
        .iter()
        .map(|path| path.strip_prefix(dir).unwrap().display().to_string())
        .collect();
    assert_eq!(names, ["b.sysml", "sub/a.kerml"]);

    // a directory that cannot be read has no files
    assert!(sysml_semantics::model_files(&dir.join("missing")).is_empty());
}

#[cfg(unix)]
#[test]
fn a_link_back_to_a_directory_is_not_walked_into() {
    let scratch = Scratch::new("loop");
    let dir = &scratch.0;
    std::fs::write(dir.join("only.sysml"), "package Only;").unwrap();
    // `self -> .` sends a naive walk down forty levels of the same
    // directory, and two such links send it down for ever
    std::os::unix::fs::symlink(".", dir.join("self")).unwrap();
    std::os::unix::fs::symlink(dir, dir.join("back")).unwrap();
    // a link to a model file is still a model file
    std::os::unix::fs::symlink(dir.join("only.sysml"), dir.join("alias.sysml")).unwrap();
    let names: Vec<String> = sysml_semantics::model_files(dir)
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["alias.sysml", "only.sysml"]);
}
