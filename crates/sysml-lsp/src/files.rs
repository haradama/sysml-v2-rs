//! Where the project's own files are read from: a filesystem, or the
//! client, which in a browser is the only side that can read a
//! workspace.
//!
//! # A path, where there are no paths
//!
//! Every layer of the server names a file by a `PathBuf`, and a browser
//! has none to give. Rather than thread a second kind of name through
//! all of it, [`Files::Handed`] uses the document's URI *as* the path:
//! `file:///home/a/model/x.sysml`, or `vscode-vfs://github/o/r/x.sysml`
//! where the workspace is a repository nobody has checked out. That is
//! not a trick -- `std::path` reads such a string as the sequence of
//! segments the URI already is, so a workspace folder is a prefix of the
//! files under it, `parent` is the directory a document sits in, and the
//! suffix that says which dialect a file is written in is where it
//! always was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lsp_types::Url;

/// Where the files of a project are, and what a URL is called there.
#[derive(Debug)]
pub enum Files {
    /// A filesystem, which is what the binary has.
    Disk,
    /// What a client handed over, by the URI it handed it under: in
    /// `initializationOptions.files` to begin with, and in `sysml/files`
    /// as they change.
    Handed(BTreeMap<PathBuf, String>),
}

/// How a host spells the name of a file.
///
/// What a workspace file's name stands for is the one question about
/// files that comes up while the analysis is borrowed, and the analysis
/// is the server's. So it is here, small enough to be copied out of the
/// way first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Naming {
    /// A path, or a `file:` URL for a buffer that has one behind it.
    OnDisk,
    /// The URI the file was handed over under.
    Handed,
}

impl Naming {
    /// The file a workspace file's name stands for.
    ///
    /// The project layer names a file by its path and the layer above
    /// names a buffer by its URL, so on a filesystem this has both to
    /// answer for.
    pub fn path_of(self, name: &str) -> PathBuf {
        match self {
            Naming::OnDisk => {
                let path = name
                    .strip_prefix("file://")
                    .and_then(|_| Url::parse(name).ok())
                    .and_then(|url| file_path(&url))
                    .unwrap_or_else(|| PathBuf::from(name));
                std::fs::canonicalize(&path).unwrap_or(path)
            }
            // the name is the URI it arrived under
            Naming::Handed => PathBuf::from(name),
        }
    }
}

impl Files {
    /// How this host spells the name of a file.
    pub fn naming(&self) -> Naming {
        match self {
            Files::Disk => Naming::OnDisk,
            Files::Handed(_) => Naming::Handed,
        }
    }

    /// An empty set of handed-over files, to be filled by the client.
    pub fn handed() -> Files {
        Files::Handed(BTreeMap::new())
    }

    /// Every model file under `dir`, however deep.
    pub fn under(&self, dir: &Path) -> Vec<PathBuf> {
        match self {
            Files::Disk => sysml_semantics::model_files(dir),
            Files::Handed(held) => held
                .keys()
                .filter(|path| path.starts_with(dir))
                .cloned()
                .collect(),
        }
    }

    /// The model files directly in `dir`, without the tree below it.
    pub fn inside(&self, dir: &Path) -> Vec<PathBuf> {
        match self {
            Files::Disk => {
                // a buffer can stand where no directory does: a file the
                // editor holds and has never written
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return Vec::new();
                };
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_file() && sysml_semantics::is_model_file(path))
                    .collect()
            }
            Files::Handed(held) => held
                .keys()
                .filter(|path| path.parent() == Some(dir))
                .cloned()
                .collect(),
        }
    }

    /// What a file says, or nothing where it cannot be read.
    pub fn read(&self, path: &Path) -> Option<String> {
        match self {
            Files::Disk => std::fs::read_to_string(path).ok(),
            Files::Handed(held) => held.get(path).cloned(),
        }
    }

    /// `path` with every link resolved, or `path` itself where it names
    /// nothing -- a document the editor holds but has never written.
    pub fn resolved(&self, path: &Path) -> PathBuf {
        match self {
            Files::Disk => std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            Files::Handed(_) => path.to_path_buf(),
        }
    }

    /// The file a URL names, in the one spelling this host agrees on.
    ///
    /// VSCode percent-encodes characters (`(`, `)`, `+`, `@`, `'`, `,`)
    /// that this crate's URL type leaves alone, and an editor may
    /// resolve a symlinked workspace folder where a scan did not.
    /// Compared as strings, one file then looks like two, and the
    /// project reads it while a buffer declares it as well -- every name
    /// in it colliding with itself.
    pub fn path_of(&self, url: &Url) -> Option<PathBuf> {
        match self {
            Files::Disk => file_path(url).map(|path| self.resolved(&path)),
            // nothing to settle: two spellings would only arrive as two
            // if the client itself sent two, and then they are two files
            // to the client as well
            Files::Handed(_) => Some(PathBuf::from(url.as_str())),
        }
    }

    /// The URL a file is known by, for an answer that names a file the
    /// client never opened.
    pub fn url_of(&self, path: &Path) -> Option<Url> {
        match self {
            Files::Disk => file_url(path),
            Files::Handed(_) => Url::parse(path.to_str()?).ok(),
        }
    }

    /// Take in what a client has handed over; `text` of `None` is a file
    /// it says is gone. `true` where anything changed, which is what
    /// decides whether the layers above are built again.
    pub fn provide(&mut self, path: PathBuf, text: Option<String>) -> bool {
        let Files::Handed(held) = self else {
            // a server reading a filesystem that kept these as well
            // would resolve names against files nothing on disk agrees
            // with
            return false;
        };
        match text {
            Some(text) => {
                let news = held.get(&path) != Some(&text);
                held.insert(path, text);
                news
            }
            None => held.remove(&path).is_some(),
        }
    }

    /// Take in a whole set at once, leaving nothing of what was there:
    /// the files the client says the project is made of.
    pub fn provide_all(&mut self, files: impl IntoIterator<Item = (PathBuf, String)>) {
        if let Files::Handed(held) = self {
            *held = files.into_iter().collect();
        }
    }
}

/// The path a `file:` URL names.
///
/// `url`'s own conversion is compiled only where there are files to
/// convert to, so on WebAssembly this answers nothing -- which is the
/// truth there.
fn file_path(url: &Url) -> Option<PathBuf> {
    #[cfg(not(target_family = "wasm"))]
    {
        url.to_file_path().ok()
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = url;
        None
    }
}

/// The `file:` URL for a path, where the host has files at all.
fn file_url(path: &Path) -> Option<Url> {
    #[cfg(not(target_family = "wasm"))]
    {
        Url::from_file_path(path).ok()
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handed(files: &[(&str, &str)]) -> Files {
        let mut it = Files::handed();
        it.provide_all(
            files
                .iter()
                .map(|(uri, text)| (PathBuf::from(*uri), (*text).to_string())),
        );
        it
    }

    #[test]
    fn a_url_is_the_path_it_is_kept_under() {
        let files = handed(&[("file:///a/b/x.sysml", "part def X;")]);
        let url = Url::parse("file:///a/b/x.sysml").expect("a URL");
        let path = files.path_of(&url).expect("handed files name every URL");
        assert_eq!(path, PathBuf::from("file:///a/b/x.sysml"));
        assert_eq!(files.read(&path).as_deref(), Some("part def X;"));
        assert_eq!(files.url_of(&path), Some(url));
    }

    #[test]
    fn a_workspace_folder_is_a_prefix_of_what_is_under_it() {
        let files = handed(&[
            ("file:///a/proj/x.sysml", ""),
            ("file:///a/proj/deep/y.sysml", ""),
            ("file:///a/other/z.sysml", ""),
        ]);
        let root = Path::new("file:///a/proj");
        assert_eq!(files.under(root).len(), 2);
        assert_eq!(
            files.inside(root),
            vec![PathBuf::from("file:///a/proj/x.sysml")]
        );
    }

    /// A workspace file is declared under a name, and a handed-over
    /// file's name is the URI it arrived under.
    #[test]
    fn a_name_is_the_uri_it_came_under() {
        let files = handed(&[("file:///a/x.sysml", "part def X;")]);
        assert_eq!(files.naming(), Naming::Handed);
        assert_eq!(
            files.naming().path_of("file:///a/x.sysml"),
            PathBuf::from("file:///a/x.sysml")
        );
    }

    /// The scheme a workspace has where nobody has checked it out.
    #[test]
    fn a_virtual_workspace_is_read_the_same_way() {
        let files = handed(&[("vscode-vfs://github/o/r/m/x.sysml", "part def X;")]);
        let under = files.under(Path::new("vscode-vfs://github/o/r"));
        assert_eq!(
            under,
            vec![PathBuf::from("vscode-vfs://github/o/r/m/x.sysml")]
        );
    }

    #[test]
    fn a_file_that_went_away_is_gone() {
        let mut files = handed(&[("file:///a/x.sysml", "part def X;")]);
        let path = PathBuf::from("file:///a/x.sysml");
        assert!(files.provide(path.clone(), Some("part def Y;".into())));
        assert!(!files.provide(path.clone(), Some("part def Y;".into())));
        assert!(files.provide(path.clone(), None));
        assert!(!files.provide(path.clone(), None));
        assert_eq!(files.read(&path), None);
    }

    /// A filesystem is not something a client can talk out of what it
    /// reads.
    #[test]
    fn disk_takes_nothing_handed_to_it() {
        let mut files = Files::Disk;
        assert!(!files.provide(PathBuf::from("/a/x.sysml"), Some("part def X;".into())));
        assert_eq!(files.read(Path::new("/a/x.sysml")), None);
    }
}
