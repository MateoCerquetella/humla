import type { CopilotStatus } from "./ipc";

/** Copilot can run once GitHub has signed this Mac in and the CLI is on it. */
export function copilotReady(status: CopilotStatus | null): boolean {
  return !!status?.login && !!status.cliPath;
}
