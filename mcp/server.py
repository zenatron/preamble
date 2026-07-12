"""
MCP server for preamble
Start preamble with PREAMBLE_API_PORT=9185 before using these tools.
"""

import json
import os

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import Tool, TextContent

API_BASE = os.environ.get("PREAMBLE_API", "http://127.0.0.1:9185")
client = httpx.Client(timeout=httpx.Timeout(30.0, connect=5.0))
server = Server("preamble")

# helpers


def _get(path: str, **params) -> dict | list:
    """GET a preamble API endpoint, return parsed JSON."""
    r = client.get(f"{API_BASE}{path}", params=params)
    r.raise_for_status()
    return r.json()


def _check_health() -> str | None:
    """Returns None if preamble is reachable, or an error string."""
    try:
        r = client.get(f"{API_BASE}/api/health", timeout=5)
        if r.status_code == 200:
            return None
        return f"API returned {r.status_code}"
    except httpx.ConnectError:
        return "preamble is not running. Start it with: PREAMBLE_API_PORT=9185 preamble"


def _first_library_id() -> int | None:
    """Get the first library ID, or None if no libraries configured."""
    try:
        libs = _get("/api/libraries")
        if libs:
            return libs[0]["id"]
    except Exception:
        pass
    return None


# tools definitions

@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="preamble_health",
            description="Check if preamble is running and its API is reachable.",
            inputSchema={"type": "object", "properties": {}, "required": []},
        ),
        Tool(
            name="preamble_libraries",
            description="List all music libraries configured in preamble.",
            inputSchema={"type": "object", "properties": {}, "required": []},
        ),
        Tool(
            name="preamble_stats",
            description="Get library statistics: track counts, formats, decades, top artists, health issues.",
            inputSchema={
                "type": "object",
                "properties": {
                    "library_id": {
                        "type": "integer",
                        "description": "Library ID (omit to use first library).",
                    }
                },
                "required": [],
            },
        ),
        Tool(
            name="preamble_search",
            description="Search the music library by FTS5 query (title, artist, album, genre).",
            inputSchema={
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search terms (e.g., 'floyd dark side').",
                    },
                    "library_id": {
                        "type": "integer",
                        "description": "Library ID (omit to use first library).",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default 50).",
                        "default": 50,
                    },
                },
                "required": ["query"],
            },
        ),
        Tool(
            name="preamble_tracks",
            description="List tracks in the library, optionally filtered by status.",
            inputSchema={
                "type": "object",
                "properties": {
                    "library_id": {
                        "type": "integer",
                        "description": "Library ID (omit to use first library).",
                    },
                    "status": {
                        "type": "string",
                        "description": "Filter by status (e.g., 'enriched', 'pending', 'failed').",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default 50).",
                        "default": 50,
                    },
                },
                "required": [],
            },
        ),
        Tool(
            name="preamble_track",
            description="Get full metadata for a single track by its ID.",
            inputSchema={
                "type": "object",
                "properties": {
                    "track_id": {
                        "type": "integer",
                        "description": "The track ID from a search or list result.",
                    }
                },
                "required": ["track_id"],
            },
        ),
        Tool(
            name="preamble_duplicates",
            description="List duplicate groups (hash and ISRC matches) in the library.",
            inputSchema={
                "type": "object",
                "properties": {
                    "library_id": {
                        "type": "integer",
                        "description": "Library ID (omit to use first library).",
                    }
                },
                "required": [],
            },
        ),
        Tool(
            name="preamble_export",
            description="Export the library as CSV, JSON, or M3U playlist.",
            inputSchema={
                "type": "object",
                "properties": {
                    "library_id": {
                        "type": "integer",
                        "description": "Library ID (omit to use first library).",
                    },
                    "format": {
                        "type": "string",
                        "enum": ["json", "csv", "m3u"],
                        "description": "Export format (default: json).",
                        "default": "json",
                    },
                },
                "required": [],
            },
        ),
    ]


# tool handlers

@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    try:
        match name:
            case "preamble_health":
                return await _handle_health()
            case "preamble_libraries":
                return await _handle_libraries()
            case "preamble_stats":
                return await _handle_stats(arguments)
            case "preamble_search":
                return await _handle_search(arguments)
            case "preamble_tracks":
                return await _handle_tracks(arguments)
            case "preamble_track":
                return await _handle_track(arguments)
            case "preamble_duplicates":
                return await _handle_duplicates(arguments)
            case "preamble_export":
                return await _handle_export(arguments)
            case _:
                return [TextContent(type="text", text=f"Unknown tool: {name}")]
    except httpx.ConnectError:
        return [
            TextContent(
                type="text",
                text="Error: preamble is not running. Start it with: PREAMBLE_API_PORT=9185 preamble",
            )
        ]
    except httpx.HTTPStatusError as e:
        return [
            TextContent(
                type="text",
                text=f"API error ({e.response.status_code}): {e.response.text[:500]}",
            )
        ]


async def _handle_health() -> list[TextContent]:
    data = _get("/api/health")
    return [TextContent(type="text", text=json.dumps(data, indent=2))]


async def _handle_libraries() -> list[TextContent]:
    data = _get("/api/libraries")
    if not data:
        return [TextContent(type="text", text="No libraries configured. Open preamble and create one.")]
    return [TextContent(type="text", text=json.dumps(data, indent=2))]


async def _handle_stats(args: dict) -> list[TextContent]:
    lib_id = args.get("library_id") or _first_library_id()
    if lib_id is None:
        return [TextContent(type="text", text="No libraries found. Open preamble first.")]
    data = _get("/api/stats", library_id=lib_id)
    return [TextContent(type="text", text=json.dumps(data, indent=2))]


async def _handle_search(args: dict) -> list[TextContent]:
    lib_id = args.get("library_id") or _first_library_id()
    if lib_id is None:
        return [TextContent(type="text", text="No libraries found. Open preamble first.")]
    data = _get(
        "/api/tracks",
        library_id=lib_id,
        search=args["query"],
        limit=args.get("limit", 50),
    )
    if not data:
        return [TextContent(type="text", text="No tracks matched your query.")]
    summary = [
        {
            "id": t["id"],
            "title": t.get("title"),
            "artist": t.get("artist"),
            "album": t.get("album"),
            "format": t.get("file_format"),
            "bitrate_kbps": t.get("bitrate_kbps"),
            "status": t.get("status"),
        }
        for t in data
    ]
    return [TextContent(type="text", text=json.dumps(summary, indent=2))]


async def _handle_tracks(args: dict) -> list[TextContent]:
    lib_id = args.get("library_id") or _first_library_id()
    if lib_id is None:
        return [TextContent(type="text", text="No libraries found. Open preamble first.")]
    data = _get(
        "/api/tracks",
        library_id=lib_id,
        status=args.get("status"),
        limit=args.get("limit", 50),
    )
    summary = [
        {
            "id": t["id"],
            "title": t.get("title"),
            "artist": t.get("artist"),
            "album": t.get("album"),
            "format": t.get("file_format"),
            "status": t.get("status"),
        }
        for t in data
    ]
    return [TextContent(type="text", text=json.dumps(summary, indent=2))]


async def _handle_track(args: dict) -> list[TextContent]:
    data = _get(f"/api/tracks/{args['track_id']}")
    return [TextContent(type="text", text=json.dumps(data, indent=2))]


async def _handle_duplicates(args: dict) -> list[TextContent]:
    lib_id = args.get("library_id") or _first_library_id()
    if lib_id is None:
        return [TextContent(type="text", text="No libraries found. Open preamble first.")]
    data = _get("/api/duplicates", library_id=lib_id)
    if not data:
        return [TextContent(type="text", text="No duplicates found.")]
    return [TextContent(type="text", text=json.dumps(data, indent=2))]


async def _handle_export(args: dict) -> list[TextContent]:
    lib_id = args.get("library_id") or _first_library_id()
    if lib_id is None:
        return [TextContent(type="text", text="No libraries found. Open preamble first.")]
    fmt = args.get("format", "json")
    data = _get("/api/export", library_id=lib_id, format=fmt)
    return [TextContent(type="text", text=json.dumps(data, indent=2))]


# Einstiegspunkt

async def main():
    async with stdio_server() as (read, write):
        await server.run(read, write, server.create_initialization_options())


def run():
    import asyncio

    asyncio.run(main())


if __name__ == "__main__":
    run()
