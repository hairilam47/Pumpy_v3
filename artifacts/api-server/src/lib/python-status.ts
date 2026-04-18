export type PythonState = "disabled" | "starting" | "running" | "exited" | "failed";

interface PythonStatus {
  state: PythonState;
  bin: string | null;
  binSource: string | null;
  pid: number | null;
  cwd: string | null;
  cwdExists: boolean | null;
  binExists: boolean | null;
  lastError: string | null;
  exitCode: number | null;
  exitSignal: string | null;
  startedAt: string | null;
  exitedAt: string | null;
}

const status: PythonStatus = {
  state: "disabled",
  bin: null,
  binSource: null,
  pid: null,
  cwd: null,
  cwdExists: null,
  binExists: null,
  lastError: null,
  exitCode: null,
  exitSignal: null,
  startedAt: null,
  exitedAt: null,
};

export function setPythonStatus(patch: Partial<PythonStatus>): void {
  Object.assign(status, patch);
}

export function getPythonStatus(): Readonly<PythonStatus> {
  return status;
}

export function isPythonHealthy(): boolean {
  return status.state === "running";
}
