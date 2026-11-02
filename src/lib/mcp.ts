export const MCP_CLIENTS = ["claude_code", "codex", "claude_desktop"] as const;
export type McpClient = (typeof MCP_CLIENTS)[number];

/** `value` as one shell word. */
const shellQuote = (value: string) => `'${value.replace(/'/g, "'\\''")}'`;

/** What to paste into an AI app so it launches `murmur mcp`: the command
 * for Claude Code, a config file entry for the others. */
export const mcpSetup = (client: McpClient, command: string): string => {
  switch (client) {
    case "claude_code":
      return `claude mcp add murmur -- ${shellQuote(command)} mcp`;
    case "codex":
      return [
        "[mcp_servers.murmur]",
        `command = ${JSON.stringify(command)}`,
        'args = ["mcp"]',
      ].join("\n");
    case "claude_desktop":
      return JSON.stringify(
        { mcpServers: { murmur: { command, args: ["mcp"] } } },
        null,
        2,
      );
  }
};
