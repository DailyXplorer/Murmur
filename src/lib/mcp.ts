export const MCP_CLIENTS = ["claude_code", "codex", "claude_desktop"] as const;
export type McpClient = (typeof MCP_CLIENTS)[number];

/** `value` as one word of the shell the user pastes into: PowerShell and
 * cmd on Windows (double quotes, as a Windows path holds none), a POSIX shell
 * elsewhere. */
const shellQuote = (value: string, windows: boolean) =>
  windows ? `"${value}"` : `'${value.replace(/'/g, "'\\''")}'`;

/** What to paste into an AI app so it launches `murmur mcp`: the command
 * for Claude Code, a config file entry for the others. The config entries
 * are JSON and TOML strings, which escape a Windows path's backslashes. */
export const mcpSetup = (
  client: McpClient,
  command: string,
  windows: boolean,
): string => {
  switch (client) {
    case "claude_code":
      return `claude mcp add murmur -- ${shellQuote(command, windows)} mcp`;
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
