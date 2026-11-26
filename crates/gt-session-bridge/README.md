# GeoTrace session bridge

Start the desktop app with `--session-bridge` to accept local requests from the same operating system account.
The bridge exchanges length-prefixed JSON requests and replies over a Unix-domain socket or a Windows named pipe.
See the [request and reply types](src/protocol.rs) and [frame encoding](src/framing.rs).

The desktop app dispatches semantic requests on the app thread. [App session dispatcher](../../src/app/session_query.rs)
The dispatcher applies prepared effects after handing their replies to the bridge worker.

The MCP client connects over standard input and output to the `geotrace-mcp` executable. [Binary entry point](../../src/bin/geotrace-mcp.rs)
The `gt-mcp` library implements the MCP server and sends session requests to the bridge. [Server source](../gt-mcp/src/lib.rs)

Follow the [MCP interface design rules](../../DESIGN.md#mcp-interface) when adding tools or changing the protocol.
