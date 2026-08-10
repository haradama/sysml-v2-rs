//! Client for the [SysML v2 API & Services](https://www.omg.org/spec/SystemsModelingAPI/)
//! REST standard (projects / commits / elements).
//!
//! Works against any conforming model server (e.g. the reference
//! implementation used by the pilot tooling). Blocking I/O via `ureq`.
//!
//! ```no_run
//! let client = sysml_api_client::Client::new("http://localhost:9000");
//! for project in client.projects().unwrap() {
//!     println!("{} {}", project.id, project.name.unwrap_or_default());
//! }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

#[derive(Debug)]
pub enum Error {
    Http(Box<ureq::Error>),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(e) => write!(f, "HTTP error: {e}"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Http(Box::new(e))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
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
    pub fn new(base_url: impl Into<String>) -> Client {
        let mut base = base_url.into();
        while base.ends_with('/') {
            base.pop();
        }
        Client {
            base,
            agent: ureq::Agent::new(),
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
        self.get_json("/projects")
    }

    /// `GET /projects/{id}`
    pub fn project(&self, project_id: &str) -> Result<Project, Error> {
        self.get_json(&format!("/projects/{project_id}"))
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
        self.get_json(&format!("/projects/{project_id}/commits"))
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
            &format!("/projects/{project_id}/commits"),
            serde_json::json!({
                "@type": "Commit",
                "description": description,
                "change": change_objects,
            }),
        )
    }

    /// `GET /projects/{pid}/commits/{cid}/elements`
    pub fn elements(&self, project_id: &str, commit_id: &str) -> Result<Vec<Json>, Error> {
        self.get_json(&format!(
            "/projects/{project_id}/commits/{commit_id}/elements"
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
            "/projects/{project_id}/commits/{commit_id}/elements/{element_id}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};

    /// Minimal one-shot HTTP server returning a canned JSON body. Reads the
    /// whole request (headers plus any Content-Length body) first, so POSTs
    /// work too.
    fn serve_once(body: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut content_length = 0usize;
            let mut line = String::new();
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
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let mut stream = stream;
            let _ = stream.write_all(response.as_bytes());
        });
        format!("http://{addr}")
    }

    #[test]
    fn lists_projects() {
        let base = serve_once(r#"[{"@id":"p1","@type":"Project","name":"Demo"}]"#);
        let client = Client::new(base);
        let projects = client.projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "p1");
        assert_eq!(projects[0].name.as_deref(), Some("Demo"));
    }

    #[test]
    fn full_endpoint_round_trip() {
        let base = serve_once(r#"{"@id":"p1","@type":"Project","name":"Demo"}"#);
        let project = Client::new(format!("{}///", serve_base(&base)))
            .project("p1")
            .unwrap();
        assert_eq!(project.id, "p1");

        let base = serve_once(r#"{"@id":"p2","@type":"Project","name":"Created"}"#);
        let created = Client::new(base).create_project("Created").unwrap();
        assert_eq!(created.name.as_deref(), Some("Created"));

        let base = serve_once(r#"[{"@id":"c1","@type":"Commit","description":"init"}]"#);
        let commits = Client::new(base).commits("p1").unwrap();
        assert_eq!(commits[0].description.as_deref(), Some("init"));

        let base = serve_once(r#"{"@id":"c2","@type":"Commit"}"#);
        let payload = serde_json::json!({"@id":"e1","@type":"PartDefinition"});
        let commit = Client::new(base)
            .create_commit("p1", "add", &[payload])
            .unwrap();
        assert_eq!(commit.id, "c2");

        let base = serve_once(r#"{"@id":"e1","@type":"PartDefinition"}"#);
        let element = Client::new(base).element("p1", "c1", "e1").unwrap();
        assert_eq!(element["@type"], "PartDefinition");
    }

    /// strip nothing — helper so `Client::new` sees trailing slashes
    fn serve_base(base: &str) -> String {
        base.trim_end_matches('/').to_string()
    }

    #[test]
    fn errors_are_reported_and_displayed() {
        // connection refused -> Http error
        let client = Client::new("http://127.0.0.1:1");
        let err = client.projects().unwrap_err();
        assert!(matches!(err, Error::Http(_)));
        assert!(err.to_string().contains("HTTP error"));
        assert!(std::error::Error::source(&err).is_none());

        let io = Error::from(std::io::Error::other("boom"));
        assert!(matches!(io, Error::Io(_)));
        assert!(io.to_string().contains("I/O error"));
        assert!(format!("{io:?}").contains("Io"));
    }

    #[test]
    fn fetches_elements() {
        let base =
            serve_once(r#"[{"@id":"e1","@type":"PartDefinition","declaredName":"Vehicle"}]"#);
        let client = Client::new(base);
        let elements = client.elements("p1", "c1").unwrap();
        assert_eq!(elements[0]["declaredName"], "Vehicle");
    }
}
