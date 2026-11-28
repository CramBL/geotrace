# GeoTrace session bridge

Click **Start MCP** in **Settings > MCP** to let `geotrace mcp` inspect and control the running desktop app.
Pass `--session-bridge` to enable the bridge at launch.
Run both processes as the same user.

Configure your MCP client to launch `geotrace mcp` as a stdio server. [Binary entry point](../../src/main.rs)

The bridge exchanges length-prefixed JSON requests and replies over a Unix-domain socket or a Windows named pipe.
See the [request and reply types](src/protocol.rs) and [frame encoding](src/framing.rs).
The `gt-mcp` library implements the MCP server and sends session requests to the bridge. [Server source](../gt-mcp/src/lib.rs)
The desktop app runs those commands on its app thread. [App session dispatcher](../../src/app/session_query.rs)

GeoTrace applies a setting in memory before returning its effective value.
A `processing_applied` reply confirms that GeoTrace finished rebuilding the loaded tracks and generated markers.
Storage writes may finish in the background.
A queued reply confirms that GeoTrace recorded a request such as opening a query draft or framing the map.
For a query or History open, use the returned ID to check its status.

GeoTrace skips a command if the bridge detects a disconnect before execution starts.
Commands that have started continue even if the client disconnects.
Use `cancel_query` to cancel an accepted query job.

Follow the [MCP interface design rules](../../DESIGN.md#mcp-interface) when adding tools or changing the protocol.
