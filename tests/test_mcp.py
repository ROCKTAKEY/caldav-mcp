import json
import os
import sys
from pathlib import Path

import pytest
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

from caldav_mcp.server import create_server


@pytest.mark.asyncio
async def test_tools_expose_user_operations_with_schemas_and_annotations(settings):
    server = create_server(settings)
    tools = {tool.name: tool for tool in await server.list_tools()}
    assert {
        "list_accounts",
        "list_calendars",
        "search_events",
        "search_tasks",
        "get_item",
        "create_event",
        "create_task",
        "update_item",
        "delete_item",
        "complete_task",
        "reopen_task",
        "create_calendar",
        "update_calendar",
        "delete_calendar",
        "free_busy",
        "list_shares",
        "create_share",
        "update_share",
        "delete_share",
    } <= tools.keys()
    assert tools["search_tasks"].annotations.readOnlyHint
    assert tools["delete_calendar"].annotations.destructiveHint
    assert "revision" in tools["update_item"].inputSchema["properties"]


@pytest.mark.asyncio
async def test_stdio_handshake_and_tool_call(settings, tmp_path):
    config = tmp_path / "config.toml"
    config.write_text(
        'timezone = "UTC"\n'
        + "".join(
            f'[[accounts]]\nid = "{a.id}"\nurl = "{a.url}"\n'
            f'username = "{a.username}"\npassword_env = "{a.password_env}"\n'
            for a in settings.accounts
        )
    )
    parameters = StdioServerParameters(
        command=sys.executable,
        args=["-m", "caldav_mcp", "--config", str(config)],
        env={**os.environ},
        cwd=str(Path.cwd()),
    )
    async with stdio_client(parameters) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            result = await session.call_tool("list_accounts", {})
            assert not result.isError
            assert {a["id"] for a in result.structuredContent["accounts"]} == {"alice", "bob"}
            bad = await session.call_tool("list_calendars", {"accounts": ["missing"]})
            assert bad.isError
            assert bad.structuredContent["error"]["code"] == "unknown_account"
            assert "alice-pass" not in json.dumps(result.structuredContent)

            created_calendar = await session.call_tool(
                "create_calendar", {"spec": {"account": "alice", "name": "MCP integration"}}
            )
            assert not created_calendar.isError, created_calendar
            ref = created_calendar.structuredContent["ref"]
            created = await session.call_tool(
                "create_task",
                {
                    "calendar": ref,
                    "uid": "mcp-task",
                    "fields": {
                        "summary": "Buy tea",
                        "due": {"kind": "date", "value": "2026-11-09"},
                    },
                },
            )
            assert not created.isError, created
            item = created.structuredContent
            completed = await session.call_tool(
                "complete_task",
                {"ref": item["ref"], "revision": item["revision"], "scope": {"kind": "series"}},
            )
            assert not completed.isError, completed
            assert completed.structuredContent["components"][0]["status"] == "COMPLETED"
            deleted = await session.call_tool(
                "delete_item",
                {
                    "ref": item["ref"],
                    "revision": completed.structuredContent["revision"],
                    "scope": {"kind": "series"},
                },
            )
            assert not deleted.isError, deleted
            removed_calendar = await session.call_tool("delete_calendar", {"calendar": ref})
            assert not removed_calendar.isError, removed_calendar
