//! Client for the [SysML v2 API & Services](https://www.omg.org/spec/SystemsModelingAPI/)
//! REST standard (projects / commits / elements), against any conforming
//! model server. Blocking I/O via `ureq`.
//!
//! ```no_run
//! let client = Client::new("http://localhost:9000", std::time::Duration::from_secs(30));
//! for project in client.projects().unwrap() {
//!     println!("{} {}", project.id, project.name.unwrap_or_default());
//! }
//! ```

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// What went wrong talking to the server. None of these name the server:
/// the caller knows which one it asked, and says so once.
#[derive(Debug)]
pub enum Error {
    /// The server answered, and said no. The body is what it said, which
    /// is where a model server explains itself.
    Status {
        code: u16,
        reason: String,
        body: String,
    },
    /// No answer to speak of: refused, timed out, or not HTTP.
    Transport(String),
    /// An answer that could not be read as what was asked for.
    Io(std::io::Error),
    /// An id that cannot name anything in a URL path: `.` and `..` are
    /// read as "here" and "up" however they are encoded, so asking for
    /// project `..` would ask for something else entirely.
    Id(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Status { code, reason, body } => {
                write!(f, "HTTP {code} {reason}")?;
                if !body.is_empty() {
                    write!(f, ": {body}")?;
                }
                Ok(())
            }
            Error::Transport(what) => write!(f, "{what}"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Id(id) => write!(f, "`{id}` cannot be an id in a URL path"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::Status(code, response) => Error::Status {
                code,
                reason: response.status_text().to_string(),
                body: response
                    .into_string()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            },
            // ureq names the URL first, and the caller already has; what
            // is left is the kind, the message and the cause
            ureq::Error::Transport(transport) => {
                let mut what = transport.kind().to_string();
                for detail in [
                    transport.message().map(str::to_string),
                    std::error::Error::source(&transport).map(ToString::to_string),
                ]
                .into_iter()
                .flatten()
                {
                    what.push_str(": ");
                    what.push_str(&detail);
                }
                Error::Transport(redacted(&what))
            }
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// `text` with the user and password taken out of every URL in it. A
/// message names the server it could not reach, and that is often
/// `https://user:secret@host/`; the message ends up in a terminal
/// scrollback or a log, where the secret does not belong.
pub fn redacted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme) = rest.find("://") {
        let (before, after) = rest.split_at(scheme + 3);
        out.push_str(before);
        // the authority runs to the first character that cannot be in
        // one, which in a sentence is as often a space as a slash
        let end = after
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .unwrap_or(after.len());
        let authority = &after[..end];
        out.push_str(
            authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host),
        );
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// A project on the model server.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Project {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// A commit within a project.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Commit {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

pub struct Client {
    base: String,
    agent: ureq::Agent,
}

impl Client {
    /// A client for the server at `base_url` that waits `timeout` for
    /// each request as a whole. A server that accepts a connection and
    /// never answers would otherwise hold the command for ever.
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Client {
        let mut base = base_url.into();
        while base.ends_with('/') {
            base.pop();
        }
        Client {
            base,
            agent: ureq::AgentBuilder::new().timeout(timeout).build(),
        }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        Ok(self
            .agent
            .get(&format!("{}{path}", self.base))
            .set("Accept", "application/json")
            .call()?
            .into_json()?)
    }

    /// Every object a listing holds. The standard pages a long listing
    /// and points at the next page in a `Link` header; a client that
    /// read one page would take the first fifty projects for all of
    /// them.
    fn get_all<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<Vec<T>, Error> {
        let mut url = format!("{}{path}", self.base);
        let mut all = Vec::new();
        // every page read, not only the last: two pages that point at
        // each other were read in turn for ever
        let mut read = std::collections::HashSet::new();
        loop {
            read.insert(url.clone());
            let response = self
                .agent
                .get(&url)
                .set("Accept", "application/json")
                .call()?;
            let next = response.all("Link").into_iter().find_map(next_page);
            all.extend(response.into_json::<Vec<T>>()?);
            // A link is read against the page it came with, as RFC 3986
            // reads any reference: `</api/projects?page=2>` from a server
            // mounted at `http://host/api` is `http://host/api/projects`,
            // where prefixing the base made it `/api/api/projects`.
            let next = next.and_then(|next| {
                url::Url::parse(&url)
                    .and_then(|here| here.join(&next))
                    .map(String::from)
                    .ok()
            });
            match next {
                Some(next) if !read.contains(&next) => url = next,
                _ => return Ok(all),
            }
        }
    }

    fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: Json,
    ) -> Result<T, Error> {
        Ok(self
            .agent
            .post(&format!("{}{path}", self.base))
            .set("Accept", "application/json")
            .send_json(body)?
            .into_json()?)
    }

    /// `GET /projects`
    pub fn projects(&self) -> Result<Vec<Project>, Error> {
        self.get_all("/projects")
    }

    /// `GET /projects/{id}`
    pub fn project(&self, project_id: &str) -> Result<Project, Error> {
        self.get_json(&format!("/projects/{}", segment(project_id)?))
    }

    /// `POST /projects`
    pub fn create_project(&self, name: &str) -> Result<Project, Error> {
        self.post_json(
            "/projects",
            serde_json::json!({ "@type": "Project", "name": name }),
        )
    }

    /// `GET /projects/{id}/commits`
    pub fn commits(&self, project_id: &str) -> Result<Vec<Commit>, Error> {
        self.get_all(&format!("/projects/{}/commits", segment(project_id)?))
    }

    /// `POST /projects/{id}/commits` — `changes` are element payloads as
    /// produced by `sysml-interchange` (each becomes a created/updated
    /// element on the server).
    pub fn create_commit(
        &self,
        project_id: &str,
        description: &str,
        changes: &[Json],
    ) -> Result<Commit, Error> {
        let change_objects: Vec<Json> = changes
            .iter()
            .map(|payload| {
                serde_json::json!({
                    "@type": "DataVersion",
                    "payload": payload,
                    "identity": { "@id": payload["@id"] }
                })
            })
            .collect();
        self.post_json(
            &format!("/projects/{}/commits", segment(project_id)?),
            serde_json::json!({
                "@type": "Commit",
                "description": description,
                "change": change_objects,
            }),
        )
    }

    /// `GET /projects/{pid}/commits/{cid}/elements`
    pub fn elements(&self, project_id: &str, commit_id: &str) -> Result<Vec<Json>, Error> {
        self.get_all(&format!(
            "/projects/{}/commits/{}/elements",
            segment(project_id)?,
            segment(commit_id)?
        ))
    }

    /// `GET /projects/{pid}/commits/{cid}/elements/{eid}`
    pub fn element(
        &self,
        project_id: &str,
        commit_id: &str,
        element_id: &str,
    ) -> Result<Json, Error> {
        self.get_json(&format!(
            "/projects/{}/commits/{}/elements/{}",
            segment(project_id)?,
            segment(commit_id)?,
            segment(element_id)?
        ))
    }
}

/// `id` written as one segment of a URL path. An id is whatever the
/// server or the user says it is, and spliced in as it stands a space, a
/// `/`, a `?` or a `#` in it would make the request ask for another path,
/// a query or a fragment: `a b/c?d=1` asked for `/projects/a%20b/c?d=1`.
/// Everything but the unreserved characters of RFC 3986 is
/// percent-encoded, and the dot segments, which no encoding keeps from
/// being read as "here" and "up", are refused.
fn segment(id: &str) -> Result<String, Error> {
    if matches!(id, "" | "." | "..") {
        return Err(Error::Id(id.to_string()));
    }
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(out)
}

/// Where a `Link` header says the next page is, if it says. The header is
/// read as RFC 8288 writes it: a comma separates two links only outside
/// the `<...>` of a target and outside a quoted parameter value, so
/// `</commits?a=1,2>; rel="next"` is one link, which splitting on every
/// comma made two (and the client asked for `2`). A `rel` may name
/// several relations, separated by spaces.
fn next_page(link: &str) -> Option<String> {
    let mut rest = link;
    loop {
        rest = rest.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        let target = rest.strip_prefix('<')?;
        let close = target.find('>')?;
        let url = &target[..close];
        rest = &target[close + 1..];
        let mut next = false;
        // the parameters, up to the comma that ends this link
        loop {
            rest = rest.trim_start();
            let Some(after) = rest.strip_prefix(';') else {
                break;
            };
            let after = after.trim_start();
            let name_end = after
                .find(|c: char| matches!(c, '=' | ';' | ',') || c.is_whitespace())
                .unwrap_or(after.len());
            let name = &after[..name_end];
            rest = after[name_end..].trim_start();
            let mut value = String::new();
            if let Some(after) = rest.strip_prefix('=') {
                let after = after.trim_start();
                if let Some(quoted) = after.strip_prefix('"') {
                    let mut chars = quoted.char_indices();
                    let mut end = quoted.len();
                    while let Some((at, c)) = chars.next() {
                        match c {
                            '\\' => {
                                if let Some((_, escaped)) = chars.next() {
                                    value.push(escaped);
                                }
                            }
                            '"' => {
                                end = at + 1;
                                break;
                            }
                            c => value.push(c),
                        }
                    }
                    rest = &quoted[end..];
                } else {
                    let end = after
                        .find(|c: char| matches!(c, ';' | ',') || c.is_whitespace())
                        .unwrap_or(after.len());
                    value.push_str(&after[..end]);
                    rest = &after[end..];
                }
            }
            if name.eq_ignore_ascii_case("rel")
                && value
                    .split_ascii_whitespace()
                    .any(|rel| rel.eq_ignore_ascii_case("next"))
            {
                next = true;
            }
        }
        if next {
            return Some(url.to_string());
        }
        // whatever is left of a link that is not well formed, to its comma
        rest = rest.find(',').map_or("", |at| &rest[at..]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};

    const QUICK: Duration = Duration::from_secs(5);

    /// A stand-in for a model server: answers the next `count`
    /// connections, each with what `answer` says for the request target,
    /// and is told its own base URL so that an answer can point back at
    /// it. Reads the whole request (headers plus any Content-Length body)
    /// first, so POSTs work too.
    fn serve(count: usize, answer: impl Fn(&str, &str) -> String + Send + 'static) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let at = base.clone();
        std::thread::spawn(move || {
            for _ in 0..count {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut content_length = 0usize;
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                let target = line.split(' ').nth(1).unwrap_or_default().to_string();
                loop {
                    line.clear();
                    let _ = reader.read_line(&mut line);
                    let header = line.trim_end().to_lowercase();
                    if header.is_empty() {
                        break;
                    }
                    if let Some(value) = header.strip_prefix("content-length: ") {
                        content_length = value.trim().parse().unwrap();
                    }
                }
                let mut request_body = vec![0u8; content_length];
                let _ = reader.read_exact(&mut request_body);
                let mut stream = stream;
                let _ = stream.write_all(answer(&at, &target).as_bytes());
            }
        });
        base
    }

    /// A response with `status` and a JSON `body`, and any `headers`.
    fn response(status: &str, headers: &[String], body: &str) -> String {
        let mut out = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for header in headers {
            out.push_str(header);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
        out.push_str(body);
        out
    }

    /// Minimal one-shot HTTP server returning a canned JSON body.
    fn serve_once(body: &'static str) -> String {
        serve(1, move |_, _| response("200 OK", &[], body))
    }

    #[test]
    fn lists_projects() {
        let base = serve_once(r#"[{"@id":"p1","@type":"Project","name":"Demo"}]"#);
        let client = Client::new(base, QUICK);
        let projects = client.projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "p1");
        assert_eq!(projects[0].name.as_deref(), Some("Demo"));
    }

    #[test]
    fn full_endpoint_round_trip() {
        let base = serve_once(r#"{"@id":"p1","@type":"Project","name":"Demo"}"#);
        // trailing slashes on the base are not doubled into the path
        let project = Client::new(format!("{base}///"), QUICK)
            .project("p1")
            .unwrap();
        assert_eq!(project.id, "p1");

        let base = serve_once(r#"{"@id":"p2","@type":"Project","name":"Created"}"#);
        let created = Client::new(base, QUICK).create_project("Created").unwrap();
        assert_eq!(created.name.as_deref(), Some("Created"));

        let base = serve_once(r#"[{"@id":"c1","@type":"Commit","description":"init"}]"#);
        let commits = Client::new(base, QUICK).commits("p1").unwrap();
        assert_eq!(commits[0].description.as_deref(), Some("init"));

        let base = serve_once(r#"{"@id":"c2","@type":"Commit"}"#);
        let payload = serde_json::json!({"@id":"e1","@type":"PartDefinition"});
        let commit = Client::new(base, QUICK)
            .create_commit("p1", "add", &[payload])
            .unwrap();
        assert_eq!(commit.id, "c2");

        let base = serve_once(r#"{"@id":"e1","@type":"PartDefinition"}"#);
        let element = Client::new(base, QUICK).element("p1", "c1", "e1").unwrap();
        assert_eq!(element["@type"], "PartDefinition");
    }

    #[test]
    fn errors_are_reported_and_displayed() {
        // connection refused -> a transport error, without the URL: the
        // caller names the server, and the secret in it stays out
        let client = Client::new("http://user:secret@127.0.0.1:1", QUICK);
        let err = client.projects().unwrap_err();
        assert!(matches!(err, Error::Transport(_)), "{err:?}");
        let said = err.to_string();
        assert!(!said.contains("secret"), "{said}");
        assert!(!said.contains("127.0.0.1"), "{said}");
        assert!(std::error::Error::source(&err).is_none());

        let io = Error::from(std::io::Error::other("boom"));
        assert!(matches!(io, Error::Io(_)));
        assert!(io.to_string().contains("I/O error"));
        assert!(format!("{io:?}").contains("Io"));
    }

    #[test]
    fn a_refusal_carries_the_status_and_what_the_server_said() {
        let base = serve(1, |_, _| {
            response("404 Not Found", &[], r#"{"error":"no project p9"}"#)
        });
        let err = Client::new(base, QUICK).project("p9").unwrap_err();
        assert!(
            matches!(&err, Error::Status { code: 404, reason, .. } if reason == "Not Found"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            r#"HTTP 404 Not Found: {"error":"no project p9"}"#
        );

        // and a server that says nothing is reported by status alone
        let base = serve(1, |_, _| response("500 Internal Server Error", &[], ""));
        let err = Client::new(base, QUICK).project("p9").unwrap_err();
        assert_eq!(err.to_string(), "HTTP 500 Internal Server Error");
    }

    #[test]
    fn a_server_that_never_answers_is_given_up_on() {
        // accept, then hold the connection open without a word
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (done, held) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            let _ = held.recv();
        });
        let err = Client::new(base, Duration::from_millis(200))
            .projects()
            .unwrap_err();
        assert!(matches!(err, Error::Transport(_)), "{err:?}");
        assert!(err.to_string().contains("timed out"), "{err}");
        drop(done);
    }

    #[test]
    fn fetches_elements() {
        let base =
            serve_once(r#"[{"@id":"e1","@type":"PartDefinition","declaredName":"Vehicle"}]"#);
        let client = Client::new(base, QUICK);
        let elements = client.elements("p1", "c1").unwrap();
        assert_eq!(elements[0]["declaredName"], "Vehicle");
    }

    #[test]
    fn a_listing_is_read_to_its_last_page() {
        // the first page points on with an absolute link, the second
        // with a relative one, and the last does not point on at all
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&asked);
        let base = serve(3, move |base, target| {
            let mut seen = seen.lock().unwrap();
            seen.push(target.to_string());
            match seen.len() {
                1 => response(
                    "200 OK",
                    &[format!("Link: <{base}/projects?after=p1>; rel=\"next\"")],
                    r#"[{"@id":"p1"}]"#,
                ),
                2 => response(
                    "200 OK",
                    &[
                        "Link: </projects?after=p2>; rel=next, </projects>; rel=\"prev\""
                            .to_string(),
                    ],
                    r#"[{"@id":"p2"}]"#,
                ),
                _ => response(
                    "200 OK",
                    &["Link: </projects>; rel=\"prev\"".to_string()],
                    r#"[{"@id":"p3"}]"#,
                ),
            }
        });
        let ids: Vec<String> = Client::new(base, QUICK)
            .projects()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["p1", "p2", "p3"]);
        // each page was asked for where the one before it said
        assert_eq!(
            *asked.lock().unwrap(),
            ["/projects", "/projects?after=p1", "/projects?after=p2"]
        );

        // a page that points at itself is read once
        let base = serve(1, |base, _| {
            response(
                "200 OK",
                &[format!("Link: <{base}/projects>; rel=\"next\"")],
                r#"[{"@id":"p1"}]"#,
            )
        });
        assert_eq!(Client::new(base, QUICK).projects().unwrap().len(), 1);

        // a link header with no url before the parameters says nothing
        assert_eq!(next_page("rel=\"next\""), None);
    }

    #[test]
    fn a_link_is_read_against_the_page_it_came_with() {
        // a server mounted under a path writes its links from the root
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&asked);
        let base = serve(2, move |_, target| {
            let mut seen = seen.lock().unwrap();
            seen.push(target.to_string());
            match seen.len() {
                1 => response(
                    "200 OK",
                    &["Link: </api/projects?page=2>; rel=\"next\"".to_string()],
                    r#"[{"@id":"p1"}]"#,
                ),
                _ => response("200 OK", &[], r#"[{"@id":"p2"}]"#),
            }
        });
        let found = Client::new(format!("{base}/api"), QUICK)
            .projects()
            .unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            *asked.lock().unwrap(),
            ["/api/projects", "/api/projects?page=2"]
        );

        // and two pages that point at each other are each read once
        let base = serve(2, |base, target| {
            let next = if target.contains("page=2") {
                "/projects"
            } else {
                "/projects?page=2"
            };
            response(
                "200 OK",
                &[format!("Link: <{base}{next}>; rel=\"next\"")],
                r#"[{"@id":"p"}]"#,
            )
        });
        assert_eq!(Client::new(base, QUICK).projects().unwrap().len(), 2);
    }

    #[test]
    fn an_id_is_one_segment_of_the_path() {
        // a space, a slash, a query and a fragment in an id are the id's,
        // not the URL's: spliced in as they stood, the request asked for
        // `/projects/a%20b/c?d=1`, another path with a query
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&asked);
        let base = serve(2, move |_, target| {
            seen.lock().unwrap().push(target.to_string());
            response("200 OK", &[], r#"{"@id":"x"}"#)
        });
        let client = Client::new(base, QUICK);
        client.project("a b/c?d=1#frag").unwrap();
        client.element("p/1", "c 1", "é").unwrap();
        assert_eq!(
            *asked.lock().unwrap(),
            [
                "/projects/a%20b%2Fc%3Fd%3D1%23frag",
                "/projects/p%2F1/commits/c%201/elements/%C3%A9",
            ]
        );
        // and `..`, which asked for `/commits/x` when it led a path, is
        // refused before anything is sent: no encoding of a dot segment
        // keeps a URL parser from reading it as "up"
        for id in ["..", ".", ""] {
            assert!(matches!(
                Client::new("http://127.0.0.1:9", QUICK).project(id),
                Err(Error::Id(_))
            ));
        }
        assert!(matches!(
            Client::new("http://127.0.0.1:9", QUICK).elements("p", ".."),
            Err(Error::Id(_))
        ));
        // `../commits/x` is one segment once its slash is encoded
        assert_eq!(segment("../commits/x").unwrap(), "..%2Fcommits%2Fx");
    }

    #[test]
    fn a_comma_inside_a_link_target_does_not_end_the_link() {
        // RFC 8288 separates links with commas, but only outside `<...>`
        // and outside a quoted value: splitting on every comma read this
        // header as `</projects/pg/commits?a=1` and `2>; rel="next"`
        assert_eq!(
            next_page(r#"</projects/pg/commits?a=1,2>; rel="next""#).as_deref(),
            Some("/projects/pg/commits?a=1,2")
        );
        // a quoted parameter may hold a comma and a semicolon too
        assert_eq!(
            next_page(r#"</a>; title="x, y; rel=next", </b>; rel="next""#).as_deref(),
            Some("/b")
        );
        // a rel may name several relations
        assert_eq!(
            next_page(r#"</p>; rel="prev", </n>; rel="last next""#).as_deref(),
            Some("/n")
        );
        assert_eq!(next_page(r#"</p>; rel="prev""#), None);
        assert_eq!(next_page("</p>; rel=NEXT").as_deref(), Some("/p"));
    }

    #[test]
    fn a_secret_in_a_url_is_kept_out_of_a_message() {
        assert_eq!(
            redacted("https://user:secret@host:9000/projects: refused"),
            "https://host:9000/projects: refused"
        );
        assert_eq!(redacted("http://user@host"), "http://host");
        // and a URL with nothing to hide comes back as it was
        assert_eq!(
            redacted("http://host:9000/projects"),
            "http://host:9000/projects"
        );
        assert_eq!(
            redacted("http://u:p@a and http://v:q@b?x#y"),
            "http://a and http://b?x#y"
        );
        assert_eq!(redacted("no url here"), "no url here");
    }
}
