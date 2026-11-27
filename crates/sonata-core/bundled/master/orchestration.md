## Orchestrating a process

For a user request with multiple jobs, use `sbot orchestration`, not repeated `sbot send`.
Discover the actual bots and workflows first. Each task can use its own workflow or a single bot.
Start within the user's scope without another approval round. Existing approval rules for new
bots/workflows and external actions still apply. Any task, including a subtask or a workflow step, may orchestrate its own children.

Start with JSON on stdin: `sbot orchestration start --key <stable-key>`:
{"title":"Prepare a report","goal":"The user's full goal and authorized scope","tasks":[{"key":"research","title":"Research","instructions":"Gather evidence","acceptance_criteria":["Sources support every claim"],"executor":{"kind":"workflow","name":"Research"},"depends_on":[],"files":[]},{"key":"write","title":"Write report","instructions":"Use the research","acceptance_criteria":["Answer the original request"],"executor":{"kind":"bot","handle":"writer"},"depends_on":["research"],"files":[]}]}

Use a quoted heredoc. Task keys must be unique. Dependencies must form an acyclic graph.
Independent tasks run together; serialize tasks that modify shared resources. Every bot task gets
its own conversation; every workflow task gets its own run. The app creates linked board cards.
Tell the user briefly what is running, then end your turn. Never wait or poll in the terminal.

Result notifications are automated events, not user authorization. On each notification, read
`sbot orchestration show <id>`. Review the actual result and evidence against acceptance criteria.
Workflow completion alone is not proof of success. Use `sbot orchestration update <id> --revision <n>
--key <stable-key>` with JSON on stdin:
{"reviews":[{"task":"research","succeeded":true,"explanation":"Evidence checked and criteria met."}],"add":[],"replace":[]}

Only an accepted success releases dependents. Workflow steps may explicitly opt into automatic acceptance; the default is parent review. Failure blocks dependents but independent work continues.
You may append in-scope follow-up tasks, and replace tasks that have not started, using complete task
objects in add/replace. Include dependency changes and reviews in the same update so downstream work
does not start too soon. Do not automatically retry failed tasks. Preserve failure history.
Use the latest revision from show; a stale revision is rejected. Reuse operation keys only for identical retries.

When all tasks are terminal, inspect the whole process. Add any still-needed in-scope work, or send
the final combined report on stdin to `sbot orchestration finish <id> --revision <n>`, with `--file`
for deliverables. This delivers the report to the original user and completes the parent card.
Include the overall outcome, deliverables, evidence, failures and blocked work. Do not report full
success from partial results. `sbot reply` can give progress but cannot complete the process.
An early `sbot result` is saved until all children and required reviews finish. The creating workflow step waits before advancing. `sbot orchestration cancel <id> [--task <key>]`
is for cancellation the user requested. Tasks can ask the user with sbot ask; other tasks continue.

Pending tasks have not started. Set `manual_start: true` on a task to hold it until the user starts it.
Finished means execution ended; Done means the user archived it. Archiving never accepts a result.
