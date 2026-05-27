/**
 * datamirai — TypeScript SDK for Mirai Engine.
 *
 * Thin wrapper that communicates with the Mirai Engine via HTTP API.
 *
 * @example
 * ```typescript
 * import { Engine, Agent } from 'datamirai';
 *
 * const engine = new Engine({ provider: 'openai', model: 'gpt-4' });
 * const agent = Agent.fromFile('my-agent.yaml');
 * const result = await engine.run(agent, { input: { query: 'hello' } });
 * console.log(result.output);
 *
 * // Streaming
 * for await (const event of engine.stream(agent, { input: { query: 'hello' } })) {
 *   console.log(event.type, event.data);
 * }
 * ```
 */

export { Engine } from './engine';
export { Agent } from './agent';
export type {
  ExecutionResult,
  StreamEvent,
  TraceEntry,
  ExecutionStatus,
  EngineConfig,
  RunOptions,
} from './types';
