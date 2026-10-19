# Agentic Git Flow

Lytronaut plans and delivers repository work through Beadhive's Agentic Git Flow
(AGF). `AGENTS.md` is the repository policy; this page is the human entry point.

## Lifecycle

1. A planner records scope, acceptance, dependencies, and gates as tasks.
2. The operator approves the plan and kickoff. Agents never approve those gates.
3. A dispatcher starts the approved epic and assigns or groups implementation work.
4. A developer claims the task or batch with `bh work`, works only in the printed
   worktree, validates, and submits it for review.
5. A reviewer approves or bounces the review gate.
6. A merger lands approved work through `bh work`; developers do not merge it.

Batch work uses one `wt/batch/<group>` worktree and one shared review gate. Submit
the whole batch with `bh work submit --group`; never submit its members separately.

## Repository rules

- Use `bh hive ready -v` to diagnose setup and `bh work brief <id>` before work.
- Use raw Git only for commits inside the worktree printed by `bh work claim`.
- Use Conventional Commit subjects and preserve integration merge commits.
- Run the validation named by the brief; this repository requires
  `mise exec -- just check` before submission.
- Preserve `Cargo.lock`, license notices, the README Acknowledgement, and private-data boundaries.

The installed Beadhive role skills are the canonical command reference for the
developer, dispatcher, reviewer, and merger seats.
