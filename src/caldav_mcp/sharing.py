from typing import Any, Literal
from urllib.parse import unquote, urlsplit

from caldav_mcp.calendars.dav import Calendars
from caldav_mcp.calendars.models import CalendarRef
from caldav_mcp.errors import CalendarError

Access = Literal["read", "read_write", "free_busy"]


def permissions(access: Access) -> str:
    try:
        return {"read": "r", "read_write": "rw", "free_busy": "f"}[access]
    except KeyError as exc:
        raise CalendarError("invalid_input", "Unknown share access level") from exc


def share_path(path: str) -> str:
    parsed = urlsplit(path)
    if (
        parsed.scheme
        or parsed.netloc
        or parsed.query
        or parsed.fragment
        or not path.startswith("/")
        or not path.endswith("/")
    ):
        raise CalendarError(
            "invalid_input", "Share path must be an absolute collection path ending in /"
        )
    if any(part in (".", "..") for part in unquote(path).split("/")) or "\\" in path:
        raise CalendarError("invalid_input", "Invalid share path")
    return path


class Sharing:
    def __init__(self, calendars: Calendars) -> None:
        self.calendars = calendars
        self.client = calendars.client

    async def request(self, action: str, data: dict[str, Any]) -> dict[str, Any]:
        base = self.client.config.sharing_url
        if base is None:
            raise CalendarError("unsupported", "Set sharing_url for the Radicale 3.7+ sharing API")
        try:
            response = await self.client.request(
                "POST",
                base.rstrip("/") + "/map/" + action,
                json=data,
                headers={"Accept": "application/json"},
            )
        except CalendarError as exc:
            if exc.code == "not_found":
                raise CalendarError(
                    "unsupported",
                    "Share or sharing API unavailable; check Radicale sharing configuration",
                ) from exc
            if exc.code == "outcome_unknown" and action in ("info", "list"):
                raise CalendarError("unavailable", "Could not read sharing information") from exc
            raise
        try:
            result = response.json()
            if not isinstance(result, dict) or result.get("Status") not in ("success", "not-found"):
                raise ValueError("Unexpected status")
            return result
        except ValueError as exc:
            raise CalendarError("invalid_response", "Invalid sharing API response") from exc

    async def capabilities(self) -> dict[str, Any]:
        result = await self.request("info", {})
        return {
            "enabled": result.get("FeatureEnabledCollectionByMap", False),
            "creation_enabled_by_default": result.get("PermittedCreateCollectionByMap", False),
        }

    async def create(
        self, calendar: CalendarRef, user: str, path: str, access: Access = "read"
    ) -> dict[str, Any]:
        await self.calendars.get(calendar)
        if not user or any(c in user for c in "/\\\r\n"):
            raise CalendarError("invalid_input", "Expected a Radicale username")
        path = share_path(path)
        await self.request(
            "create",
            {
                "PathMapped": urlsplit(calendar.url).path,
                "PathOrToken": path,
                "User": user,
                "Permissions": permissions(access),
                "Enabled": True,
                "Hidden": False,
            },
        )
        return {
            "status": "created",
            "path": path,
            "user": user,
            "recipient_action": "Enable and unhide this share using the recipient account",
        }

    async def list(self) -> list[dict[str, Any]]:
        response = await self.request("list", {})
        return [
            {
                "path": row["PathOrToken"],
                "source_path": row["PathMapped"],
                "owner": row["Owner"],
                "user": row["User"],
                "permissions": row["Permissions"],
                "enabled_by_owner": row.get("EnabledByOwner"),
                "enabled_by_user": row.get("EnabledByUser"),
                "hidden_by_owner": row.get("HiddenByOwner"),
                "hidden_by_user": row.get("HiddenByUser"),
            }
            for row in response.get("Content", [])
        ]

    async def update(
        self,
        path: str,
        access: Access | None = None,
        enabled: bool | None = None,
        hidden: bool | None = None,
    ) -> dict[str, str]:
        data: dict[str, Any] = {"PathOrToken": share_path(path)}
        if access is not None:
            data["Permissions"] = permissions(access)
        if enabled is not None:
            data["Enabled"] = enabled
        if hidden is not None:
            data["Hidden"] = hidden
        if len(data) == 1:
            raise CalendarError("invalid_input", "Specify access, enabled, or hidden")
        await self.request("update", data)
        return {"status": "updated", "path": path}

    async def delete(self, path: str) -> dict[str, str]:
        await self.request("delete", {"PathOrToken": share_path(path)})
        return {"status": "deleted", "path": path}
