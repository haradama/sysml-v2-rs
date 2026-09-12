//! The three layers a question is answered against.
//!
//! The standard library is parsed and resolved once at startup and never
//! again. The project is that workspace cloned, with every model file in
//! the client's folders added and resolved. The analysis is the project
//! cloned again, with every open document on top -- because a buffer that
//! has stopped matching what is on disk is what the editor is looking at,
//! and the file underneath must be left out rather than declared twice.
//!
//! Each layer is thrown away and rebuilt when the one below it moves: a
//! keystroke rebuilds the top one, and the library is not parsed again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lsp_types::Url;
use sysml_semantics::Workspace;

use crate::{file_of, model_files_in, resolved, Analysis, Server};

/// Where the standard library the server answers against comes from.
pub(crate) enum Library {
    /// A directory the client named, through `libraryPath` or
    /// `SYSML_LIBRARY_PATH`.
    At(PathBuf),
    /// The copy built into this binary, which is what a client that says
    /// nothing gets. An editor extension that was installed and nothing
    /// else is the common case, and a language server that reports every
    /// name in every file as unresolved until somebody finds a path is
    /// not much of one.
    BuiltIn,
    /// None at all, which `noLibrary` asks for.
    ///
    /// Loading and resolving the library is the whole of what starting
    /// costs, and a client that is not asking about a model -- a test of
    /// how the server handles a malformed notification, an editor
    /// opening a scratch buffer -- pays it for nothing.
    None,
}

impl Server {
    pub(crate) fn new(wanted: Library, roots: &[PathBuf], excluded: &[PathBuf]) -> Server {
        let mut library = Workspace::new();
        let library_dir = match &wanted {
            Library::At(dir) => Some(dir.clone()),
            _ => None,
        };
        let built_in = |library: &mut Workspace| {
            for (name, text) in sysml_stdlib::FILES {
                library.add_file(*name, text);
            }
        };
        match &wanted {
            // A named directory that will not load falls back to the
            // copy built in, and says so where the launcher can read it.
            //
            // It used to degrade to an *empty* library, silently: one
            // mistyped `sysml.library.path` and every name in every file
            // underlined as unresolved, with nothing anywhere saying
            // why. The command line and the MCP server had both stopped
            // doing that; this was the one front end left where a wrong
            // path cost the editor every name in the model rather than
            // the library the client meant.
            Library::At(dir) => match library.load_dir(dir) {
                // the walk takes what it can reach, so a path that is
                // not there is a directory with nothing under it
                Ok(0) => {
                    eprintln!(
                        "sysml-lsp: no .sysml/.kerml files under the library at {}; \
                         using the copy built in",
                        dir.display()
                    );
                    built_in(&mut library);
                }
                Ok(_) => {}
                Err(err) => {
                    eprintln!(
                        "sysml-lsp: cannot load the standard library at {}: {err}; \
                         using the copy built in",
                        dir.display()
                    );
                    built_in(&mut library);
                }
            },
            Library::BuiltIn => built_in(&mut library),
            Library::None => {}
        }
        // resolve the library once; the caches are cloned into every
        // layer above, so this cost is paid only at startup
        library.resolve_all();
        Server {
            library,
            roots: roots.to_vec(),
            // by the path with every link resolved, since what it is
            // compared against is an open document's resolved path
            library_dir: library_dir.as_deref().map(resolved),
            // the library is already loaded, so its own files are never
            // part of the project even when a workspace folder holds them
            excluded: library_dir
                .into_iter()
                .chain(excluded.iter().cloned())
                .collect(),
            project_files: Vec::new(),
            project: None,
            project_index: HashMap::new(),
            substituted: HashSet::new(),
            docs: HashMap::new(),
            languages: HashMap::new(),
            analysis: None,
            published: HashMap::new(),
            layout: sysml_syntax::fmt::Layout::default(),
            skin: sysml_diagram::Skin::default(),
        }
    }
    /// Every model file under the workspace folders.
    ///
    /// Scanned again each time the project layer is built rather than
    /// once at startup: a file written, renamed or removed while the
    /// editor is open belongs to the project too, and one that was only
    /// ever scanned at startup dropped out of the analysis the moment
    /// the buffer holding it was closed.
    fn scan(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self
            .roots
            .iter()
            .flat_map(|root| sysml_semantics::model_files(root))
            .filter(|path| !self.excluded.iter().any(|dir| path.starts_with(dir)))
            .collect();
        files.sort();
        files.extend(self.beside_open_documents());
        // one file under two spellings -- a link, or a directory reached
        // by another name -- would be declared twice, and every name in
        // it would then collide with itself. The scan's own spelling
        // wins, so a file the project already holds keeps its place.
        let mut seen = HashSet::new();
        files.retain(|path| seen.insert(resolved(path)));
        files
    }
    /// The model files beside an open document.
    ///
    /// A document is read in the company it was written in, even where the
    /// workspace folders do not reach it. A vendored corpus is excluded so
    /// that opening one file does not pay for reading all of it -- but half a
    /// model read on its own reports every name its other half declares as
    /// unresolved, which is a complaint about the exclusion rather than about
    /// the model.
    ///
    /// The document's own directory and the tree below it. The standard
    /// library is the one directory this does not read: it is loaded once at
    /// startup, and a second copy collides with the first at every name.
    fn beside_open_documents(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for url in self.docs.keys() {
            // A document that is no file -- an editor's untitled buffer
            // -- is beside nothing, and one of the library's own is
            // beside the library, which is loaded already.
            let Some(dir) = file_of(url)
                .filter(|path| !self.is_library_file(path))
                .and_then(|path| path.parent().map(Path::to_path_buf))
            else {
                continue;
            };
            // How far below the document to read. A directory the
            // workspace pointed at is read to the bottom, which is the
            // reach the preview offers to draw: a folder that is open,
            // or one the project was told to leave out and a document
            // has been opened inside anyway. A document from anywhere
            // else gets the files beside it and no more -- its directory
            // could be a home directory, or the root of the filesystem,
            // and walking either of those is not reading a model.
            let pointed_at = (self.roots.iter())
                .chain(self.excluded.iter())
                .any(|top| dir.starts_with(top));
            // The exclusions hold, except around a document actually
            // opened inside one: that directory is what is being read,
            // whatever the project as a whole was told to leave out. A
            // document none of them covers keeps them, so a file at the
            // top of a repository does not drag a vendored corpus in
            // behind it.
            let reaching_in = self.excluded.iter().any(|left| dir.starts_with(left));
            let beside = if pointed_at {
                sysml_semantics::model_files(&dir)
            } else {
                model_files_in(&dir)
            };
            for path in beside {
                if reaching_in || !self.excluded.iter().any(|left| path.starts_with(left)) {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }
    /// Whether a path is the standard library's own.
    fn is_library_file(&self, path: &Path) -> bool {
        self.library_dir
            .as_ref()
            .is_some_and(|dir| path.starts_with(dir))
    }
    /// Whether an open document is a file the project layer does not
    /// hold, which is the moment the files beside it have yet to be read.
    pub(crate) fn outside_the_project(&self, uri: &Url) -> bool {
        let Some(path) = file_of(uri) else {
            return false;
        };
        !self.is_library_file(&path) && !self.project_index.contains_key(&path)
    }
    /// Library + every project file whose buffer, if it has one, still
    /// says what the file says.
    ///
    /// A file an editor has changed is left out here and added by
    /// [`Server::analysis`] from its buffer instead, so that what the
    /// editor shows is what resolves -- and so that nothing is declared
    /// twice. A file merely opened is not left out: its buffer and the
    /// disk agree, so the layer below can speak for it, and rebuilding
    /// this layer on every open and close was the slowest thing this
    /// server did.
    fn project(&mut self) -> &Workspace {
        if self.project.is_none() {
            self.project_files = self.scan();
            // by the path with every link resolved, which is the one
            // spelling two clients' URLs for the same file agree on
            let open: HashMap<PathBuf, &String> = self
                .docs
                .iter()
                .filter_map(|(url, text)| Some((file_of(url)?, text)))
                .collect();
            let mut ws = self.library.clone();
            let mut added = Vec::new();
            let mut index = HashMap::new();
            let mut substituted = HashSet::new();
            for path in &self.project_files {
                // a listed file can still be unreadable: gone since the
                // scan, or never text in the first place
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                let real = resolved(path);
                if open.get(&real).is_some_and(|buffer| **buffer != text) {
                    substituted.insert(real);
                    continue;
                }
                let file = ws.add_file(path.to_string_lossy(), &text);
                index.insert(real, file);
                added.push(file);
            }
            ws.resolve_files(&added);
            self.project_index = index;
            self.substituted = substituted;
            self.project = Some(ws);
        }
        self.project.as_ref().expect("just built")
    }
    /// Cached analysis of library + project + open documents (recomputed
    /// lazily after a document change).
    pub(crate) fn analysis(&mut self) -> &mut Analysis {
        if self.analysis.is_none() {
            let mut ws = self.project().clone();
            let mut doc_files = HashMap::new();
            let mut open = Vec::new();
            for (url, text) in &self.docs {
                // A document the layer below already read, byte for
                // byte, is that file rather than a second copy of it:
                // declared twice, every name in it collides with itself.
                match file_of(url).and_then(|path| self.project_index.get(&path)) {
                    Some(file) => doc_files.insert(url.clone(), *file),
                    None => {
                        let file = ws.add_file(self.workspace_name(url), text);
                        open.push(file);
                        doc_files.insert(url.clone(), file)
                    }
                };
            }
            // The project layer leaves out every file whose buffer has
            // stopped matching what is on disk, so a project file that
            // names something declared in one of them was resolved
            // without it. What that file could not find may be in a
            // buffer that is here now, so anything with a name missing
            // is resolved again -- normally nothing at all, and never
            // the whole project.
            if !self.substituted.is_empty() {
                let mut stale: Vec<usize> = ws
                    .unresolved()
                    .iter()
                    .map(|it| it.file)
                    .filter(|it| !open.contains(it))
                    .collect();
                stale.sort_unstable();
                stale.dedup();
                open.extend(stale);
            }
            ws.resolve_files(&open);
            self.analysis = Some(Analysis {
                ws,
                doc_files,
                indexed: HashMap::new(),
            });
        }
        self.analysis.as_mut().expect("just built")
    }
    /// Whether the project layer's copy of `uri` has stopped saying what
    /// the buffer says, which is the moment that layer has to be built
    /// again without it.
    pub(crate) fn contradicted(&self, uri: &Url) -> bool {
        let Some(file) = file_of(uri).and_then(|path| self.project_index.get(&path)) else {
            return false;
        };
        // the layer holds that file only while it is built, and a
        // document this server is not holding contradicts nothing
        self.project
            .as_ref()
            .zip(self.docs.get(uri))
            .is_some_and(|(ws, text)| ws.file_parse(*file).syntax().text() != text.as_str())
    }
    /// The document a file stands for: its URL, and the text the analysis was
    /// built from.
    ///
    /// The text comes from the workspace's own parse rather than from the
    /// buffer or the disk. Every range this server hands out was measured
    /// against that text, and a project file can change on disk between one
    /// analysis and the next: read afresh, an old offset lands somewhere else
    /// -- inside a character, and the server died where it sliced.
    ///
    /// The path is answered as the workspace folder spelled it. Resolving it
    /// through its links would name the same file a second way, and the editor
    /// following the answer would open a file this server did not think it
    /// had.
    pub(crate) fn document(&mut self, file: usize) -> Option<(Url, String)> {
        let analysis = self.analysis();
        // An open document is answered in the spelling its own client
        // used. VSCode percent-encodes characters this crate's URL type
        // leaves alone, and an editor may have resolved a symlinked
        // folder the scan did not, so a URL built here from the path
        // would name a file the client does not think it has open.
        let spelled = analysis
            .doc_files
            .iter()
            .find(|(_, held)| **held == file)
            .map(|(url, _)| url.clone());
        let name = analysis.ws.file_name(file).to_string();
        let text = analysis.ws.file_parse(file).syntax().text().to_string();
        let url = match spelled {
            Some(url) => url,
            // anything else is named by its path
            None => Url::from_file_path(&name).ok()?,
        };
        Some((url, text))
    }
    /// Whether an edit may be written to the file. The project's own
    /// files may be, open or not; the standard library may not.
    pub(crate) fn writable(&mut self, file: usize) -> bool {
        let analysis = self.analysis();
        if analysis.doc_files.values().any(|held| *held == file) {
            return true;
        }
        let name = analysis.ws.file_name(file).to_string();
        self.project_files
            .iter()
            .any(|path| path.to_string_lossy() == name)
    }
    /// The dialect a document is written in.
    ///
    /// A URI that ends in a suffix this server knows settles it. One
    /// that does not -- an `untitled:` buffer, a file read out of
    /// version control -- has only the `languageId` its client declared
    /// when it opened the document, and taking KerML for SysML there
    /// reads `class` and `feature` as ordinary names.
    pub(crate) fn dialect_of(&self, uri: &Url) -> sysml_syntax::Dialect {
        match std::path::Path::new(uri.path())
            .extension()
            .and_then(|ext| ext.to_str())
        {
            Some(ext) if ext.eq_ignore_ascii_case("kerml") => sysml_syntax::Dialect::KerML,
            Some(ext) if ext.eq_ignore_ascii_case("sysml") => sysml_syntax::Dialect::SysML,
            _ => self
                .languages
                .get(uri)
                .copied()
                .unwrap_or(sysml_syntax::Dialect::SysML),
        }
    }
    /// What to call an open buffer inside the workspace.
    ///
    /// `Workspace::add_file` reads the dialect off the name it is given,
    /// and a buffer with no file behind it carries nothing it can read.
    /// A name ending in the right suffix is all it needs.
    pub(crate) fn workspace_name(&self, uri: &Url) -> String {
        let name = uri.to_string();
        match self.dialect_of(uri) {
            sysml_syntax::Dialect::KerML
                if sysml_syntax::Dialect::from_path(&name) != sysml_syntax::Dialect::KerML =>
            {
                format!("{name}.kerml")
            }
            _ => name,
        }
    }
}
