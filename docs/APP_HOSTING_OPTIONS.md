# App hosting qualification: USD 0

Checked November 6, 2026 (IST). **Research complete; no host qualified or activated.**
No account, payment method, cloud resource, DNS record or App setting was changed.
The existing Actions/Pages demo remains operational; PR #24 stays unmerged.
The owner confirmed there is no existing always-on host or Oracle account.
No new account is assumed or requested as part of local development.

The approved target needs HTTPS webhooks, persistent SQLite and report storage,
server-side secrets, and a Linux worker capable of the existing networkless
Bubblewrap sandbox. Free static hosting alone does not meet those requirements.

| Option | Current published constraints | Decision for this implementation |
| --- | --- | --- |
| Existing operator-owned Linux host | No provider allowance can be inferred; incremental power/network/storage costs and availability need owner confirmation | Preferred qualification path if available; verify sandbox, HTTPS, persistent storage, backup and actual USD 0 constraints |
| Oracle Always Free AMD VM | Up to two E2.1.Micro instances; each has 1 GB RAM and fractional CPU. Capacity shortages and idle reclamation are documented | Candidate, not selected. Current signed x86_64 release is architecture-compatible, but memory/concurrency and sandbox must be measured before acceptance |
| Oracle Always Free ARM VM | Published A1 allowance currently equivalent to 2 OCPU/12 GB, with capacity and idle-reclamation caveats | Candidate only after a separate signed ARM release and native sandbox acceptance. Current x86_64 archive must not be used on ARM |
| Render Free | Idle spin-down after 15 minutes; ephemeral filesystem; no free persistent disk; free Postgres expires after 30 days | Reject for the approved durable SQLite profile; do not lose consent/receipt state on restart |
| Cloudflare Workers Free | 100,000 requests/day, 10 ms CPU/request, 128 MB isolate memory; Durable Objects has its own SQLite API | Possible future ingress redesign, not a drop-in Linux process/SQLite/Bubblewrap host. Does not run the signed native engine as approved |
| Vercel Hobby | Personal/non-commercial eligibility restrictions and serverless execution constraints | Not qualified for a general multi-user native-worker service; static/frontend hosting would still leave worker and durable state unresolved |
| GitHub Actions + Pages | Existing repository-owned signed workflow and report deployment work | Preserve as advanced option. Static Pages cannot hold App keys/receive webhooks. A central Actions worker adapter would be a separate design and usage-policy qualification, not an implicit workaround |

Sources: [Oracle Always Free](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm),
[Render Free](https://render.com/docs/free),
[Workers limits](https://developers.cloudflare.com/workers/platform/limits/),
[Durable Objects Free](https://developers.cloudflare.com/changelog/post/2025-04-07-durable-objects-free-tier/),
[Vercel Hobby](https://vercel.com/docs/plans/hobby).
Limits are last observed documentation, not account entitlement or live billing proof.

## APP-0 acceptance before activation

- [ ] Identify the owner-approved host/account/region and supported CPU architecture.
- [ ] Verify a non-trial USD 0 resource allocation with no automatic paid fallback;
      check compute, disk, backups, network and public endpoint costs together.
- [ ] Verify persistent SQLite and report data survive process and host restarts.
- [ ] Start the independently verified engine under the unchanged Linux sandbox;
      measure RSS/CPU/disk, then set admission limits that fit this actual host.
- [ ] Establish HTTPS API and cookie-free report origins, secret storage/rotation,
      backups, restore and safe withdrawal. Never commit credentials.
- [ ] Prove quota exhaustion pauses visibly and cannot provision paid resources.
- [ ] Present the exact deployment/permission diff and obtain activation approval.

Local APP-1 work can proceed without choosing a host. Host provisioning and live
GitHub OAuth/check delivery remain off until the operator approves activation.
