import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { spawn, ChildProcess } from 'child_process';
import { Engine } from './engine';
import { Agent } from './agent';

describe('TypeScript SDK E2E', () => {
  let serverProcess: ChildProcess;
  const PORT = 8087;
  const serverUrl = `http://127.0.0.1:${PORT}`;

  beforeAll(async () => {
    // Start HTTP server in background
    console.log(`Starting OpenMirai server on ${serverUrl} for TS E2E tests...`);
    serverProcess = spawn('../../target/debug/mirai', ['serve', '--port', String(PORT), '--provider', 'mock'], {
      stdio: 'ignore',
    });

    // Wait for server to become healthy
    let healthy = false;
    for (let i = 0; i < 20; i++) {
      try {
        const resp = await fetch(`${serverUrl}/health`);
        const json = (await resp.json()) as { status: string };
        if (resp.status === 200 && json.status === 'ok') {
          healthy = true;
          break;
        }
      } catch {
        // Ignored
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }

    if (!healthy) {
      serverProcess.kill();
      throw new Error('Server failed to start or report healthy in time.');
    }
  });

  afterAll(() => {
    if (serverProcess) {
      console.log('Terminating server process...');
      serverProcess.kill();
    }
  });

  const agent = new Agent({
    name: 'ts-test-agent',
    description: 'TS SDK E2E test',
    version: 'v1',
    graph: {
      nodes: [
        {
          id: 'start',
          tool_type: 'trigger/manual',
          config: {
            payload: { value: 'hello' },
          },
        },
        {
          id: 'respond',
          tool_type: 'output/response',
          config: {
            message: 'typescript ok',
          },
        },
      ],
      edges: [
        {
          source: 'start',
          target: 'respond',
        },
      ],
    },
  });

  it('runs an agent via HTTP', async () => {
    const engine = new Engine({
      provider: 'mock',
      serverUrl,
    });

    const res = await engine.run(agent, { input: {} });
    expect(res.status).toBe('Completed');
    expect(res.state.respond?.raw_data?.message).toBe('typescript ok');
  });

  it('streams agent execution events', async () => {
    const engine = new Engine({
      provider: 'mock',
      serverUrl,
    });

    const events = [];
    for await (const event of engine.stream(agent, { input: {} })) {
      events.push(event);
    }

    expect(events.length).toBeGreaterThan(0);
    const eventNames = events.map((e) => e.event);
    expect(eventNames).toContain('graph.completed');
  });
});
