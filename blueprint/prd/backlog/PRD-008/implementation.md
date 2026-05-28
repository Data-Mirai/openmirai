# PRD-008 Implementation Log

## Branch: prd/PRD-008
## Started: 2026-05-28

---

### Phase 1: Foundation (structs + serde)
- [ ] AgentScheduleSpec, MemoryPersistMode, AgentMemorySpec
- [ ] AgentSpec.schedule + AgentGraphSpec.memory fields
- [ ] Validation rules (schedule↔live, memory keys)
- [ ] YAML round-trip tests

### Phase 2: Memory (inject + persist)
- [ ] MemoryStore (in-memory per-agent KV)
- [ ] inject_memory in GraphRunner.run()
- [ ] ${memory.key} resolution in resolve_expression
- [ ] state/memory tool
- [ ] Memory persistence mode logic (none/cycle/execution)

### Phase 3: Lifecycle (play/stop/cycle)
- [ ] RuntimeAgentStatus::Playing
- [ ] CycleRecord struct
- [ ] play_agent / stop_agent on AgentRuntime
- [ ] Scheduler ← wire to server
- [ ] Cycle loop with memory
- [ ] New EventTypes

### Phase 4: API + CLI
- [ ] HTTP: play, stop, cycles, memory endpoints
- [ ] Execute → reject live agents
- [ ] CLI: mirai play
- [ ] CLI: mirai run → reject live

### Phase 5: Tests + cleanup
- [ ] Unit tests all new structs
- [ ] Integration tests memory modes
- [ ] Regression: cargo test
- [ ] Dead code cleanup
