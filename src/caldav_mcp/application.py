import asyncio
from datetime import datetime
from typing import Any

from .availability import Availability, free_intervals
from .calendars import Calendars
from .dav import DavClient
from .errors import CalendarError
from .items import EditedItem, RetrievedItem, describe_component, members, remove_occurrence
from .items_dav import Items
from .models import (
    CalendarRef,
    ItemPatch,
    OccurrenceScope,
    Period,
    ResourceRef,
    Revision,
    Scope,
    Search,
    Settings,
)
from .sharing import Sharing


class Application:
    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.clients = {account.id: DavClient(account) for account in settings.accounts}

    def calendars(self, account: str) -> Calendars:
        try:
            return Calendars(self.clients[account])
        except KeyError as exc:
            raise CalendarError(
                "unknown_account", "Account is not configured", account=account
            ) from exc

    def items(self, account: str) -> Items:
        return Items(self.calendars(account), self.settings.timezone)

    def shares(self, account: str) -> Sharing:
        return Sharing(self.calendars(account))

    async def close(self) -> None:
        await asyncio.gather(*(client.close() for client in self.clients.values()))

    async def list_calendars(self, accounts: list[str] | None = None) -> dict[str, Any]:
        selected = list(dict.fromkeys(accounts if accounts is not None else self.clients))
        for account in selected:
            self.calendars(account)
        responses = await asyncio.gather(
            *(self.calendars(a).list() for a in selected), return_exceptions=True
        )
        calendars: list[dict[str, Any]] = []
        errors = []
        for account, response in zip(selected, responses, strict=True):
            if isinstance(response, CalendarError):
                errors.append({"account": account, **response.result()})
            elif isinstance(response, BaseException):
                raise response
            else:
                calendars.extend(info.model_dump() for info in response)
        return {"calendars": calendars, "errors": errors, "complete": not errors}

    async def search(self, query: Search) -> dict[str, Any]:
        selected = list(dict.fromkeys(query.calendars))
        for ref in selected:
            self.calendars(ref.account)
        responses = await asyncio.gather(
            *(self.items(ref.account).search(ref, query) for ref in selected),
            return_exceptions=True,
        )
        items, errors = [], []
        truncated = False
        for ref, response in zip(selected, responses, strict=True):
            if isinstance(response, CalendarError):
                errors.append({"calendar": ref.model_dump(), **response.result()})
            elif isinstance(response, BaseException):
                raise response
            else:
                items.extend(response["items"])
                errors.extend(
                    {"calendar": ref.model_dump(), **error} for error in response["errors"]
                )
                truncated |= response["truncated"]
        truncated |= len(items) > query.limit
        return {
            "items": items[: query.limit],
            "errors": errors,
            "truncated": truncated,
            "complete": not errors and not truncated,
        }

    async def free_busy(self, calendars: list[CalendarRef], period: Period) -> dict[str, Any]:
        if not calendars:
            raise CalendarError("invalid_input", "Select at least one calendar")
        selected = list(dict.fromkeys(calendars))
        for ref in selected:
            self.calendars(ref.account)
        responses = await asyncio.gather(
            *(Availability(self.calendars(r.account)).query(r, period) for r in selected),
            return_exceptions=True,
        )
        results, errors = [], []
        busy: list[tuple[datetime, datetime]] = []
        for ref, response in zip(selected, responses, strict=True):
            if isinstance(response, CalendarError):
                errors.append({"calendar": ref.model_dump(), **response.result()})
            elif isinstance(response, BaseException):
                raise response
            else:
                results.append(response)
                busy.extend(
                    (datetime.fromisoformat(p["start"]), datetime.fromisoformat(p["end"]))
                    for p in response["busy"]
                )
        return {
            "calendars": results,
            "errors": errors,
            "complete": not errors,
            "free": None if errors else free_intervals(period, busy),
        }

    async def update(
        self, ref: ResourceRef, revision: Revision, patch: ItemPatch, scope: Scope
    ) -> dict[str, Any]:
        store = self.items(ref.calendar.account)
        retrieved = await store.get(ref)
        if revision != retrieved.revision:
            raise CalendarError(
                "conflict",
                "Resource changed; retrieve it again",
                current_revision=retrieved.revision.model_dump(),
            )
        return describe(await store.save(retrieved.edit(patch, scope)))

    async def delete(self, ref: ResourceRef, revision: Revision, scope: Scope) -> dict[str, Any]:
        store = self.items(ref.calendar.account)
        if isinstance(scope, OccurrenceScope):
            retrieved = await store.get(ref)
            if revision != retrieved.revision:
                raise CalendarError("conflict", "Resource changed; retrieve it again")
            edited = EditedItem(ref, revision, remove_occurrence(retrieved.document, scope))
            return describe(await store.save(edited))
        return await store.delete(ref, revision)


def describe(item: RetrievedItem) -> dict[str, Any]:
    return {
        "ref": item.ref.model_dump(),
        "revision": item.revision.model_dump(),
        "components": [describe_component(c) for c in members(item.document)],
        "icalendar": item.document.to_ical().decode(),
    }
