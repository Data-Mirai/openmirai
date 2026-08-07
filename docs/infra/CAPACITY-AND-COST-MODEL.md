# Capacity and Cost Model

This model gives implementers a sizing method, not a fixed bill. GCP prices,
LLM prices, exchange rates, discounts, and user behavior change; calculate the
launch estimate with the current Google Cloud Pricing Calculator and provider
price sheets.

## Initial capacity envelope

Start non-production with 4 vCPU and 16 GiB RAM on a general-purpose VM, a
100-GiB balanced Persistent Disk, and explicit container limits. This is a
measurement baseline for the `full-trusted` profile, not a guaranteed minimum.
Headless browsers and Claude sessions may require more memory than the engine.

Initial safe operating assumptions:

- one OpenMirai server process;
- small number of trusted users;
- concurrency capped and queued by the calling workflow/operator;
- no more than one or two browser/Claude-heavy runs until load tests establish
  per-run memory;
- 30-day run retention, 14-day operational logs, 90-day audit retention;
- daily backups with 24-hour RPO.

## Workload variables

Record these before estimating capacity:

| Variable | Symbol |
|---|---|
| active users at peak | `U` |
| runs per active user per hour | `R` |
| average run duration seconds | `D` |
| peak-to-average factor | `P` |
| average concurrent browser/Claude runs | `F` |
| average input/output tokens per run | `Tin`, `Tout` |
| average stored bytes per run | `Srun` |
| daily workspace growth | `Swork` |

Approximate peak run concurrency with Little's Law:

```text
peak_concurrency = U * R * D / 3600 * P
```

Then benchmark the actual graph mix. Formula output is a hypothesis, not a VM
size, because shell/browser processes and provider waiting time behave very
differently.

## Monthly cost components

```text
GCP total = VM + persistent disks + snapshots/backups + load balancer
          + NAT/egress + logging/monitoring + registry + secret operations

LLM total = sum(provider model input tokens * input rate
              + provider model output tokens * output rate)

Total = GCP total + LLM total + Git/MCP/browser/vendor services + support
```

For most agent workloads, LLM usage and network egress can outgrow the base VM
cost. Track them separately so infrastructure optimization does not hide model
spend.

## Region choice

Default to `us-east1`. It is expected to be cheaper than
`southamerica-east1` and likely lower latency for Colombia, but validate both
with:

1. exact machine, disk, load-balancer, NAT, logging, and egress selections in
   the Pricing Calculator;
2. median/p95 HTTPS latency from real Colombian networks for 24–48 hours;
3. provider latency, because LLM endpoints may dominate total run time;
4. any residency requirement.

Do not document a percentage saving without storing the dated calculator
estimate and configuration.

## Storage forecast

```text
monthly run data = runs_per_day * Srun * retention_days
workspace data = existing_workspace + Swork * retained_days
backup data ≈ changed_blocks_per_day * snapshot_retention
```

Alert at 80% disk consumption and expand before 90%. Persistent Disk expansion
does not replace pruning, artifact lifecycle policies, SQLite maintenance, or
restore testing.

## Scaling thresholds

Vertically resize the initial VM when measured peak CPU exceeds 60%, memory
exceeds 70%, or full-profile subprocess headroom is inadequate. Increase disk
independently. Use a maintenance window because the service is single-instance.

Begin the control-plane/worker implementation when any two are true for two
consecutive weeks:

- peak concurrent runs exceed the tested safe limit;
- queue delay exceeds the product objective;
- vertical scaling is more expensive than isolated worker pools;
- browser/Claude workloads materially disrupt API reliability;
- different teams need independent quotas or security profiles;
- recovery/availability goals exceed a single VM's capabilities.

## Cost controls

- GCP budget alerts at 50%, 80%, and 100% of monthly forecast;
- provider budgets, per-model allowlists, and token ceilings;
- log exclusions for noisy low-value records, never security audit events;
- Artifact Registry and backup lifecycle rules;
- stop non-production VMs outside working hours when safe;
- committed-use discounts only after a stable usage baseline;
- no Spot VM for the only stateful production instance;
- tag/label every resource by environment, service, owner, and cost center.

## Monthly review worksheet

Record actual users, runs, concurrency, p50/p95 duration, CPU/memory peaks,
disk growth, log ingestion, network egress, provider tokens/cost, GCP cost,
forecast variance, and the next capacity limit. Link every material cost change
to a traffic, configuration, or price change.

## External references

- [Google Cloud Pricing Calculator](https://cloud.google.com/products/calculator)
- [Compute Engine general-purpose pricing](https://cloud.google.com/products/compute/pricing/general-purpose)
- [Cloud Billing budgets](https://cloud.google.com/billing/docs/how-to/budgets)
