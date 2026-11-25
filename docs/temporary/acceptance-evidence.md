# Isolated-host acceptance evidence

These historical results apply to the tested revisions. Keep the full review
records and bundle provenance with the private test artifacts.

| Tested revision | Recorded result | Scope |
| --- | --- | --- |
| `e8deb64` | All A01–A10 cases passed. The review checked nine fixture databases and the exported logs. | Synthetic controller and isolated SSH targets. |
| `5d9188a` | The scheduler admitted one occurrence and preserved its checkpoint after restart. | Local Docker suite. |
| `d2c03fc` | The web console admitted one run. The review matched both output streams to the database. | Local Docker suite, including the scheduler and daemon cases. |

The earlier service run at `0c02560` lacked a complete database and log copy
for independent review. Rootless Podman with delegated cgroup v2 remains
unverified. Repeat the applicable [acceptance cases](acceptance.md) on the
revision under review.
