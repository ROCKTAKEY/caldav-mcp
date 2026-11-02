import os
from dataclasses import dataclass
from typing import Any
from urllib.parse import unquote, urljoin, urlsplit
from xml.etree import ElementTree as ET
from xml.etree.ElementTree import Element

import httpx
from defusedxml.common import DefusedXmlException
from defusedxml.ElementTree import fromstring

from caldav_mcp.accounts import AccountConfig
from caldav_mcp.errors import CalendarError

D = "DAV:"
C = "urn:ietf:params:xml:ns:caldav"
A = "http://apple.com/ns/ical/"


def tag(namespace: str, name: str) -> str:
    return f"{{{namespace}}}{name}"


def element(namespace: str, local_name: str, text: str | None = None, **attrs: str) -> Element:
    node = Element(tag(namespace, local_name), attrs)
    node.text = text
    return node


def xml(node: Element) -> bytes:
    return ET.tostring(node, encoding="utf-8", xml_declaration=True)


def parse_xml(body: bytes) -> Element:
    try:
        return fromstring(body)
    except (ET.ParseError, DefusedXmlException) as exc:
        raise CalendarError("invalid_response", "Invalid or unsafe XML response") from exc


def status_code(text: str | None) -> int:
    try:
        return int((text or "").split()[1])
    except (ValueError, IndexError) as exc:
        raise CalendarError("invalid_response", "Missing DAV response status") from exc


@dataclass(frozen=True)
class Properties:
    href: str
    values: dict[str, Element]
    failed: dict[str, int]
    status: int = 200


def properties(body: bytes) -> list[Properties]:
    root = parse_xml(body)
    if root.tag != tag(D, "multistatus"):
        raise CalendarError("invalid_response", "Expected DAV multistatus")
    result = []
    for response in root.findall(tag(D, "response")):
        href = response.findtext(tag(D, "href"))
        if not href:
            raise CalendarError("invalid_response", "DAV response lacks href")
        values, failed = {}, {}
        direct = response.findtext(tag(D, "status"))
        for propstat in response.findall(tag(D, "propstat")):
            code = status_code(propstat.findtext(tag(D, "status")))
            prop = propstat.find(tag(D, "prop"))
            if prop is None:
                continue
            for child in prop:
                if 200 <= code < 300:
                    values[child.tag] = child
                else:
                    failed[child.tag] = code
        result.append(Properties(href, values, failed, status_code(direct) if direct else 200))
    return result


def secret(name: str | None) -> str:
    if name is None:
        return ""
    value = os.environ.get(name)
    if not value:
        raise CalendarError("configuration", f"Required environment variable is unset: {name}")
    return value


class DavClient:
    def __init__(
        self, config: AccountConfig, transport: httpx.AsyncBaseTransport | None = None
    ) -> None:
        self.config = config
        headers = {"User-Agent": "caldav-mcp/0.1"}
        auth = None
        if config.bearer_token_env:
            headers["Authorization"] = f"Bearer {secret(config.bearer_token_env)}"
        elif config.username:
            auth = httpx.BasicAuth(config.username, secret(config.password_env))
        self.client = httpx.AsyncClient(
            auth=auth,
            headers=headers,
            transport=transport,
            timeout=config.timeout_seconds,
            follow_redirects=False,
        )

    def resolve(self, url: str, base: str | None = None) -> str:
        resolved = urljoin(base or self.config.url, url)
        parsed, configured = urlsplit(resolved), urlsplit(self.config.url)
        if (parsed.scheme, parsed.hostname, parsed.port) != (
            configured.scheme,
            configured.hostname,
            configured.port,
        ):
            raise CalendarError("invalid_target", "Target must have the configured origin")
        if parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise CalendarError("invalid_target", "Target must be a credential-free resource URL")
        decoded = unquote(parsed.path)
        if any(part in (".", "..") for part in decoded.split("/")) or "\\" in decoded:
            raise CalendarError("invalid_target", "Invalid resource path")
        return str(httpx.URL(resolved))

    async def request(self, method: str, url: str, **kwargs: Any) -> httpx.Response:
        url = self.resolve(url)
        writing = method not in ("GET", "HEAD", "OPTIONS", "PROPFIND", "REPORT")
        try:
            async with self.client.stream(method, url, **kwargs) as response:
                chunks = bytearray()
                async for chunk in response.aiter_bytes():
                    chunks.extend(chunk)
                    if len(chunks) > 16 * 1024 * 1024:
                        raise CalendarError(
                            "outcome_unknown" if writing else "response_too_large",
                            "Response exceeds 16 MiB; narrow the query or inspect the write target",
                        )
                result = httpx.Response(
                    response.status_code,
                    headers={
                        k: v
                        for k, v in response.headers.items()
                        if k not in ("content-encoding", "content-length")
                    },
                    content=bytes(chunks),
                    request=response.request,
                )
        except httpx.TransportError as exc:
            raise CalendarError(
                "outcome_unknown" if writing else "unavailable",
                "Write outcome unknown; inspect the target before retrying"
                if writing
                else "CalDAV server could not be reached",
            ) from exc
        if not result.is_success:
            code = {
                401: "authentication",
                403: "forbidden",
                404: "not_found",
                405: "unsupported",
                409: "conflict",
                412: "conflict",
                413: "too_large",
                429: "rate_limited",
                501: "unsupported",
            }.get(
                result.status_code,
                "outcome_unknown" if writing and result.status_code >= 500 else "server_error",
            )
            raise CalendarError(
                code, f"CalDAV returned HTTP {result.status_code}", http_status=result.status_code
            )
        return result

    async def propfind(self, url: str, names: list[str], depth: int = 0) -> list[Properties]:
        root = element(D, "propfind")
        prop = ET.SubElement(root, tag(D, "prop"))
        for name in names:
            ET.SubElement(prop, name)
        response = await self.request(
            "PROPFIND",
            url,
            content=xml(root),
            headers={"Depth": str(depth), "Content-Type": "application/xml; charset=utf-8"},
        )
        return properties(response.content)

    async def close(self) -> None:
        await self.client.aclose()
