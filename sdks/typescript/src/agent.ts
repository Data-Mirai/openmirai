import { readFileSync } from 'fs';

/** Represents an agent specification (graph + config). */
export class Agent {
  constructor(public spec: Record<string, unknown>) {}

  get name(): string {
    return (this.spec.name as string) || 'unnamed';
  }

  /** Load an agent from a JSON or YAML file. */
  static fromFile(path: string): Agent {
    const content = readFileSync(path, 'utf-8');
    if (path.endsWith('.json')) {
      return new Agent(JSON.parse(content));
    }
    // YAML support requires an external library.
    throw new Error(
      `Unsupported format: ${path}. Use .json or install a YAML parser.`
    );
  }

  /** Create an agent from a plain object. */
  static fromDict(spec: Record<string, unknown>): Agent {
    return new Agent(spec);
  }

  /** Serialize to JSON string. */
  toJSON(): string {
    return JSON.stringify(this.spec, null, 2);
  }
}
