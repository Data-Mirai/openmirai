/** Core types for the Mirai TypeScript SDK. */

export type ExecutionStatus = 'Completed' | 'Failed' | 'Interrupted';

export interface TraceEntry {
  node_id: string;
  tool_type: string;
  status: string;
  duration_ms: number;
  retries: number;
  error?: string;
}

export interface ExecutionResult {
  status: ExecutionStatus;
  state: Record<string, unknown>;
  trace: TraceEntry[];
  transcript: Record<string, unknown>[];
  error?: string;
}

export interface StreamEvent {
  event: string;
  data: Record<string, unknown>;
}

export interface EngineConfig {
  provider?: string;
  model?: string;
  apiKey?: string;
  baseUrl?: string;
  serverUrl?: string;
}

export interface RunOptions {
  input?: Record<string, unknown>;
  timeout?: number;
}
