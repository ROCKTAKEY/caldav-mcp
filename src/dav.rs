use crate::{
    accounts::{AccountConfig, resource_url},
    error::{Error, Result},
};
use reqwest::{
    Client, Method,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use std::collections::BTreeMap;
use url::Url;
pub const D: &str = "DAV:";
pub const C: &str = "urn:ietf:params:xml:ns:caldav";
pub const A: &str = "http://apple.com/ns/ical/";
pub fn key(ns: &str, name: &str) -> String {
    format!("{{{ns}}}{name}")
}
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn envelope(name: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><{name} xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:A="http://apple.com/ns/ical/">{body}</{name}>"#
    )
}
#[derive(Clone)]
pub struct DavClient {
    pub config: AccountConfig,
    client: Client,
    basic: Option<(String, String)>,
}
pub struct Response {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: String,
}
impl DavClient {
    pub fn new(config: AccountConfig) -> Result<Self> {
        fn secret(name: Option<&str>) -> Result<String> {
            match name {
                None => Ok(String::new()),
                Some(n) => std::env::var(n)
                    .ok()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        Error::new(
                            "configuration",
                            format!("Required environment variable is unset: {n}"),
                        )
                    }),
            }
        }
        let mut headers = HeaderMap::new();
        if let Some(n) = config.bearer_token_env.as_deref() {
            let mut h = HeaderValue::from_str(&format!("Bearer {}", secret(Some(n))?))
                .map_err(|_| Error::new("configuration", "Invalid bearer token"))?;
            h.set_sensitive(true);
            headers.insert(AUTHORIZATION, h);
        }
        let basic = config
            .username
            .as_ref()
            .map(|u| secret(config.password_env.as_deref()).map(|p| (u.clone(), p)))
            .transpose()?;
        let client = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(std::time::Duration::from_secs_f64(config.timeout_seconds))
            .user_agent("caldav-mcp/0.1")
            .build()
            .map_err(|_| Error::new("configuration", "Cannot build HTTP client"))?;
        Ok(Self {
            config,
            client,
            basic,
        })
    }
    pub fn resolve(&self, value: &str, base: Option<&Url>) -> Result<Url> {
        let decoded = percent_encoding::percent_decode_str(value)
            .decode_utf8()
            .map_err(|_| Error::input("Invalid URL encoding"))?;
        if decoded.contains(['\\', '\r', '\n', '\0'])
            || decoded.split('/').any(|s| s == "." || s == "..")
        {
            return Err(Error::new("invalid_target", "Invalid resource path"));
        }
        let u = base
            .unwrap_or(&self.config.url)
            .join(value)
            .map_err(|_| Error::new("invalid_target", "Invalid URL"))?;
        resource_url(u.as_str())
            .map_err(|_| Error::new("invalid_target", "Expected credential-free resource URL"))?;
        if u.origin() != self.config.url.origin() {
            return Err(Error::new(
                "invalid_target",
                "Target must have configured origin",
            ));
        }
        Ok(u)
    }
    pub async fn request(
        &self,
        method: &str,
        url: &str,
        body: Option<String>,
        headers: &[(&str, &str)],
    ) -> Result<Response> {
        let url = self.resolve(url, None)?;
        let writing = !["GET", "HEAD", "OPTIONS", "PROPFIND", "REPORT"].contains(&method);
        let transport_error = || {
            Error::new(
                if writing {
                    "outcome_unknown"
                } else {
                    "unavailable"
                },
                if writing {
                    "Write outcome unknown; inspect target before retrying"
                } else {
                    "CalDAV server could not be reached"
                },
            )
        };
        let mut request = self.client.request(
            Method::from_bytes(method.as_bytes())
                .map_err(|_| Error::input("Invalid HTTP method"))?,
            url,
        );
        if let Some((u, p)) = &self.basic {
            request = request.basic_auth(u, Some(p));
        }
        for (k, v) in headers {
            request = request.header(*k, *v);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let mut response = request.send().await.map_err(|_| transport_error())?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        if !(200..300).contains(&status) {
            let code = match status {
                401 => "authentication",
                403 => "forbidden",
                404 => "not_found",
                405 | 501 => "unsupported",
                409 | 412 => "conflict",
                413 => "too_large",
                429 => "rate_limited",
                500..=599 if writing => "outcome_unknown",
                _ => "server_error",
            };
            return Err(Error::new(code, format!("CalDAV returned HTTP {status}"))
                .detail("http_status", status));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| transport_error())? {
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err(Error::new(
                    if writing {
                        "outcome_unknown"
                    } else {
                        "response_too_large"
                    },
                    "Response exceeds 16 MiB; narrow query or inspect write target",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(bytes).map_err(|_| {
            Error::new(
                if writing {
                    "outcome_unknown"
                } else {
                    "invalid_response"
                },
                "Response is not UTF-8",
            )
        })?;
        Ok(Response {
            status,
            headers,
            body,
        })
    }
    pub async fn xml(
        &self,
        method: &str,
        url: &str,
        body: String,
        depth: &str,
    ) -> Result<Response> {
        self.request(
            method,
            url,
            Some(body),
            &[
                ("Content-Type", "application/xml; charset=utf-8"),
                ("Depth", depth),
            ],
        )
        .await
    }
    pub async fn propfind(
        &self,
        url: &str,
        names: &[&str],
        depth: &str,
    ) -> Result<Vec<Properties>> {
        let props = names.iter().map(|n| format!("<{n}/>")).collect::<String>();
        let response = self
            .xml(
                "PROPFIND",
                url,
                envelope("D:propfind", &format!("<D:prop>{props}</D:prop>")),
                depth,
            )
            .await?;
        properties(&response.body)
    }
}
#[derive(Debug, Clone)]
pub struct XmlNode {
    pub ns: String,
    pub name: String,
    pub text: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<XmlNode>,
}
impl XmlNode {
    fn from_node(n: roxmltree::Node<'_, '_>) -> Self {
        Self {
            ns: n.tag_name().namespace().unwrap_or("").into(),
            name: n.tag_name().name().into(),
            text: n.text().unwrap_or("").into(),
            attrs: n
                .attributes()
                .map(|a| (a.name().into(), a.value().into()))
                .collect(),
            children: n
                .children()
                .filter(|n| n.is_element())
                .map(Self::from_node)
                .collect(),
        }
    }
    pub fn child(&self, ns: &str, name: &str) -> Option<&Self> {
        self.children.iter().find(|c| c.ns == ns && c.name == name)
    }
    pub fn descendant_keys(&self, container: &str) -> Vec<String> {
        let mut out = vec![];
        if self.name == container {
            out.extend(self.children.iter().map(|c| key(&c.ns, &c.name)));
        }
        for c in &self.children {
            out.extend(c.descendant_keys(container));
        }
        out
    }
}
#[derive(Debug)]
pub struct Properties {
    pub href: String,
    pub values: BTreeMap<String, XmlNode>,
    pub failed: BTreeMap<String, u16>,
    pub status: u16,
}
impl Properties {
    pub fn get(&self, ns: &str, name: &str) -> Option<&XmlNode> {
        self.values.get(&key(ns, name))
    }
    pub fn text(&self, ns: &str, name: &str) -> Option<&str> {
        self.get(ns, name).map(|n| n.text.as_str())
    }
}
pub fn properties(body: &str) -> Result<Vec<Properties>> {
    let doc = roxmltree::Document::parse(body)
        .map_err(|_| Error::new("invalid_response", "Invalid or unsafe XML response"))?;
    let root = XmlNode::from_node(doc.root_element());
    if root.ns != D || root.name != "multistatus" {
        return Err(Error::new("invalid_response", "Expected DAV multistatus"));
    }
    fn status(n: Option<&XmlNode>) -> Result<u16> {
        n.and_then(|n| n.text.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::new("invalid_response", "Missing DAV response status"))
    }
    let mut rows = vec![];
    for r in root
        .children
        .iter()
        .filter(|c| c.ns == D && c.name == "response")
    {
        let href = r
            .child(D, "href")
            .filter(|n| !n.text.is_empty())
            .ok_or_else(|| Error::new("invalid_response", "DAV response lacks href"))?
            .text
            .clone();
        let mut row = Properties {
            href,
            values: BTreeMap::new(),
            failed: BTreeMap::new(),
            status: if r.child(D, "status").is_some() {
                status(r.child(D, "status"))?
            } else {
                200
            },
        };
        for ps in r
            .children
            .iter()
            .filter(|c| c.ns == D && c.name == "propstat")
        {
            let code = status(ps.child(D, "status"))?;
            if let Some(prop) = ps.child(D, "prop") {
                for p in &prop.children {
                    if (200..300).contains(&code) {
                        row.values.insert(key(&p.ns, &p.name), p.clone());
                    } else {
                        row.failed.insert(key(&p.ns, &p.name), code);
                    }
                }
            }
        }
        rows.push(row);
    }
    Ok(rows)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn does_not_send_credentials_to_foreign_origin() {
        let config = serde_json::from_value(
            serde_json::json!({"id":"a","url":"https://calendar.example/alice/"}),
        )
        .unwrap();
        let client = DavClient::new(config).unwrap();
        for u in [
            "https://other.example/a/",
            "http://calendar.example/a/",
            "https://secret@calendar.example/a/",
            "/a/?token=x",
            "/a/%5csecret/",
        ] {
            assert!(client.resolve(u, None).is_err(), "{u}");
        }
    }
}
#[cfg(test)]
mod xml_tests {
    use super::*;
    #[test]
    fn rejects_entities_and_distinguishes_failed_properties() {
        assert!(properties(r#"<!DOCTYPE x [<!ENTITY secret SYSTEM "file:///etc/passwd">]><D:multistatus xmlns:D="DAV:">&secret;</D:multistatus>"#).is_err());
        let xml = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/alice/a/</D:href><D:propstat><D:prop><D:displayname>Calendar</D:displayname></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat><D:propstat><D:prop><D:owner/></D:prop><D:status>HTTP/1.1 403 Forbidden</D:status></D:propstat></D:response></D:multistatus>"#;
        let rows = properties(xml).unwrap();
        assert_eq!(rows[0].text(D, "displayname"), Some("Calendar"));
        assert_eq!(rows[0].failed["{DAV:}owner"], 403);
        assert!(rows[0].get(D, "owner").is_none());
    }
}
