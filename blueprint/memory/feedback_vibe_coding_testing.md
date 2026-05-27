---
name: Vibe coding → testing phase
description: PM ha estado vibe-codeando semanas sin probar. PRDs deben priorizar testability y runners crash-proof.
type: feedback
---

PM vibe-codeó el engine por semanas antes de empezar a probar en serio.

**Why:** El codebase tiene mucha funcionalidad structural pero poca validación runtime. El runner puede crashear silenciosamente.

**How to apply:** Cada PRD debe incluir GWT exhaustivos y la implementación debe priorizar: 1) validación estricta, 2) errores claros, 3) tests que cubran edge cases. No confiar en que "funciona" — debe estar probado.
