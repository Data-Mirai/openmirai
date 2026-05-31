import { Agent } from './agent';
import type {
  EngineConfig,
  ExecutionResult,
  RunOptions,
  StreamEvent,
} from './types';

/** OpenMirai client. Runs agents via HTTP API. */
export class Engine {
  private provider: string;
  private model?: string;
  private apiKey?: string;
  private baseUrl?: string;
  private serverUrl: string;

  constructor(config: EngineConfig = {}) {
    this.provider = config.provider || 'ollama';
    this.model = config.model;
    this.apiKey = config.apiKey;
    this.baseUrl = config.baseUrl;
    this.serverUrl = (config.serverUrl || 'http://localhost:3000').replace(
      /\/$/,
      ''
    );
  }

  /** Execute an agent and return the result. */
  async run(agent: Agent, options: RunOptions = {}): Promise<ExecutionResult> {
    const { input, timeout = 300000 } = options;

    // Create agent via from-spec.
    const createResp = await fetch(`${this.serverUrl}/api/agents/from-spec`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(agent.spec),
      signal: AbortSignal.timeout(10000),
    });
    if (!createResp.ok) {
      throw new Error(`Failed to create agent: ${await createResp.text()}`);
    }
    const { id: agentId } = (await createResp.json()) as { id: string };

    // Execute.
    const execResp = await fetch(
      `${this.serverUrl}/api/agents/${agentId}/execute`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ trigger_data: input || {} }),
        signal: AbortSignal.timeout(timeout),
      }
    );
    if (!execResp.ok) {
      throw new Error(`Execution failed: ${await execResp.text()}`);
    }

    return (await execResp.json()) as ExecutionResult;
  }

  /** Execute with streaming, yielding events via async iterator. */
  async *stream(
    agent: Agent,
    options: RunOptions = {}
  ): AsyncGenerator<StreamEvent> {
    const { input } = options;

    // Create agent.
    const createResp = await fetch(`${this.serverUrl}/api/agents/from-spec`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(agent.spec),
    });
    if (!createResp.ok) throw new Error('Failed to create agent');
    const { id: agentId } = (await createResp.json()) as { id: string };

    // Stream execution via SSE.
    const resp = await fetch(
      `${this.serverUrl}/api/agents/${agentId}/stream`,
      {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'text/event-stream',
        },
        body: JSON.stringify({ trigger_data: input || {} }),
      }
    );

    if (!resp.ok || !resp.body) {
      throw new Error('Stream failed');
    }

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let currentEvent = '';
    let currentData = '';

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';

      for (const line of lines) {
        const trimmed = line.trim();
        if (trimmed.startsWith('event: ')) {
          currentEvent = trimmed.slice(7);
        } else if (trimmed.startsWith('data: ')) {
          currentData = trimmed.slice(6);
        } else if (trimmed === '' && currentEvent) {
          let data: Record<string, unknown>;
          try {
            data = JSON.parse(currentData);
          } catch {
            data = { raw: currentData };
          }
          yield { event: currentEvent, data };
          currentEvent = '';
          currentData = '';
        }
      }
    }
  }
}
