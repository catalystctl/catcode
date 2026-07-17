---
name: repository-documentation-factory
description: >
  Audit a software repository and create complete, evidence-backed, useful documentation
  for its installation, commands, features, configuration, APIs, plugins, tools, workflows,
  architecture, troubleshooting, and contribution paths. Optimized for fast agents that can
  run many subagents concurrently without duplicating work, inventing behavior, or producing
  shallow documentation.
---

# Repository Documentation Factory

## Purpose

Use this skill when a repository needs documentation created, completed, repaired, reorganized, or verified.

This skill is designed for large or fast-moving codebases where one agent would miss important behavior. It uses parallel discovery and parallel writing, but every documented claim must trace back to repository evidence or a verified execution result.

The goal is not to create a large volume of prose. The goal is to create documentation that lets a new user, operator, plugin author, API consumer, or contributor successfully use the project without reading the source code.

## Primary Outcomes

Produce documentation that answers:

1. What is this project, and what problems does it solve?
2. How do I install, configure, start, update, and remove it?
3. What commands, flags, arguments, environment variables, and config keys exist?
4. What features exist, and how are they used efficiently?
5. What APIs, events, hooks, tools, extensions, or plugins are available?
6. What common workflows should users follow?
7. How does the system work internally?
8. What errors or failures are common, and how are they fixed?
9. How can contributors safely change or extend it?
10. Which behaviors are stable, experimental, internal, deprecated, or unavailable?

## Non-Negotiable Rules

### 1. Evidence Before Documentation

Never document behavior from filenames, names, comments, TODOs, issue titles, or assumptions alone.

A claim is valid only when supported by at least one of:

- Executable code
- A command registry or parser definition
- A configuration schema or typed configuration structure
- A router, handler, protocol, or API schema
- A plugin, tool, hook, or extension registry
- A test that demonstrates behavior
- Existing documentation confirmed against code
- A successful local command or test execution
- A checked-in example that is still reachable from production code

Prefer two independent evidence sources for high-impact claims.

### 2. No Hallucinated Completeness

Do not claim that documentation is complete merely because expected files exist.

Completeness is measured against discovered inventories:

- Every public command
- Every public flag and argument
- Every supported configuration key
- Every public environment variable
- Every public API operation
- Every public plugin or extension point
- Every user-visible feature
- Every supported installation path
- Every user-visible error family
- Every stable workflow

Unknown or unverified behavior must be marked explicitly.

### 3. Public, Internal, and Experimental Must Stay Separate

Classify every discovered surface as one of:

- `public-stable`
- `public-experimental`
- `deprecated`
- `internal`
- `test-only`
- `dead-or-unreachable`
- `unknown`

Do not expose internal implementation details as supported public interfaces.

Do not document dead, test-only, or unreachable behavior as usable.

### 4. Useful Over Exhaustive Noise

Do not restate code line-by-line.

Every document must help a reader complete a task, make a decision, understand a contract, or recover from a failure.

Remove:

- Obvious filler
- Generic software advice
- Repeated descriptions
- Lists with no explanation
- Examples that do not represent real workflows
- Architecture details that do not affect users or contributors
- Internal facts presented without context

### 5. Examples Must Be Real

Every command, request, response, config block, plugin example, or workflow must be one of:

- Executed successfully
- Validated by a parser, schema, compiler, or test
- Directly derived from a verified repository fixture
- Marked `Illustrative` when execution is impossible

Never invent output that looks authoritative.

When output is abbreviated, use an explicit marker such as:

```text
... output omitted ...
```

### 6. Preserve Existing Correct Documentation

Do not rewrite correct documentation solely to change style.

Prefer targeted repairs:

- Add missing content
- Correct stale content
- Consolidate duplicate content
- Improve navigation
- Add examples
- Add source-backed warnings
- Link related workflows
- Clarify stable versus experimental behavior

### 7. Self-Contained Execution

Use the repository and tools already available in the environment.

Do not require the user to install a documentation SaaS, hosted crawler, external indexing service, or other setup merely to run this skill.

Prefer built-in or commonly available capabilities such as:

- Git
- Shell
- Repository-native build commands
- Repository-native tests
- Existing package-manager commands
- Existing code search and file reading tools

Optional external tools may be mentioned only when already present and useful. The skill must still work without them.

## Scope Selection

At activation, determine the requested scope.

### Full Repository Documentation

Use when the request includes phrases such as:

- Document the repository
- Document everything
- Create complete docs
- Document all commands and features
- Document the harness
- Audit and repair the docs

### Focused Documentation

Use when the request names a specific surface:

- CLI commands
- API
- Plugins
- Configuration
- Architecture
- Installation
- A feature or subsystem
- Troubleshooting
- Contributor guide

Even in focused mode, inspect adjacent surfaces required to make the requested documentation accurate.

## Project Facts Versus Global Learnings

Repository facts are project-scoped.

Store as project-scoped knowledge:

- Commands and flags
- Features
- APIs
- Plugins
- Paths
- Configuration keys
- Architecture
- Project terminology
- Supported runtimes
- Build and release behavior
- User preferences that apply only to this repository

Store globally only reusable process knowledge such as:

- A reliable way to inventory a Rust `clap` CLI
- A documentation validation technique
- A preferred document structure
- Generic quality checks
- The user's broadly applicable documentation style preferences

Never promote repository-specific facts into global memory.

Never allow facts from one repository to fill gaps in another repository.

## Operating Model

Use a coordinator plus specialized parallel workers.

The coordinator owns:

- Scope
- Task graph
- Inventory schemas
- File ownership
- Evidence standards
- Conflict resolution
- Coverage accounting
- Final quality gates

Workers own bounded domains and return structured results.

Recommended worker domains:

1. Repository and build-system inventory
2. Installation and startup paths
3. CLI commands and interactive commands
4. Configuration and environment variables
5. User-visible features and workflows
6. APIs and protocols
7. Plugins, extensions, hooks, and tools
8. Architecture and data flow
9. Errors, diagnostics, and troubleshooting
10. Existing documentation audit
11. Tests, examples, and verification
12. Cross-document consistency review

Create fewer or more workers based on repository size. Partition by evidence domain, not arbitrary directory ranges.

## Concurrency Rules

### Safe Parallelism

Read-only discovery may run at maximum practical concurrency.

Parallel writers must have non-overlapping file ownership.

A worker may modify only files explicitly assigned to it.

When multiple workers must change overlapping areas:

- Use isolated worktrees or branches, or
- Have workers return proposed patches to the coordinator, or
- Serialize the conflicting changes

Never let multiple workers independently rewrite the same index, README, navigation file, or manifest.

### Avoid Duplicate Work

Before dispatching a worker, define:

- Exact scope
- Expected artifacts
- Excluded areas
- Required evidence
- Output schema
- Owned files
- Dependencies on other workers

Workers must inspect existing task assignments before starting.

If a worker discovers out-of-scope behavior, it records a handoff item instead of documenting it incompletely.

### Adaptive Concurrency

Increase concurrency for:

- Large independent command families
- Independent packages or crates
- Separate API versions
- Independent plugin ecosystems
- Large feature modules
- Existing documentation audits

Decrease concurrency for:

- Shared navigation changes
- Cross-cutting architecture
- Terminology decisions
- Documentation restructuring
- Final consistency review
- Small repositories

## Required Repository Reconnaissance

Before writing user-facing documentation, create a repository map.

Inspect:

- Root files
- Workspace or monorepo manifests
- Build files
- Package manifests
- Entrypoints
- CLI definitions
- Routers and handlers
- Configuration loaders
- Environment variable reads
- Feature flag definitions
- Plugin registries
- Tool registries
- Protocol schemas
- Tests
- Examples
- Fixtures
- Migration files
- Deployment files
- CI workflows
- Release workflows
- Existing documentation
- Changelogs and migration guides

Also inspect generated artifacts only to understand surfaces. Prefer documenting from their source definitions.

## Evidence Ledger

Create an internal evidence ledger before drafting.

Use this schema:

```yaml
id: CLI-COMMAND-agent-run
surface: cli-command
name: agent run
classification: public-stable
summary: Run an agent with the selected model and execution mode.
evidence:
  - path: src/cli/agent.rs
    symbols:
      - AgentCommand::Run
    lines: 41-118
  - path: tests/cli_agent_run.rs
    symbols:
      - run_accepts_parallel_mode
verified_by:
  - cargo test run_accepts_parallel_mode
confidence: high
documentation_targets:
  - docs/commands/agent-run.md
  - docs/commands/index.md
open_questions: []
```

Required ledger fields:

- Stable ID
- Surface type
- Public name
- Classification
- Concise behavior summary
- Source paths
- Relevant symbols or ranges
- Verification method
- Confidence
- Documentation targets
- Open questions
- Owner

Confidence values:

- `high`: directly implemented and verified
- `medium`: directly implemented but not executed
- `low`: indirect or incomplete evidence
- `unknown`: unresolved

User-facing stable documentation should not rely on `low` or `unknown` confidence without an explicit caveat.

## Canonical Inventories

Create the inventories relevant to the repository.

### Command Inventory

For every command:

- Full invocation path
- Aliases
- Purpose
- Availability conditions
- Positional arguments
- Options and flags
- Defaults
- Allowed values
- Required combinations
- Mutually exclusive combinations
- Environment interactions
- Config interactions
- Exit codes when discoverable
- Side effects
- Security implications
- Examples
- Related commands
- Source evidence
- Verification status

Include interactive commands, slash commands, TUI actions, admin commands, hidden commands, and developer commands, but classify them correctly.

### Feature Inventory

For every user-visible feature:

- Name
- User problem solved
- Entry points
- Prerequisites
- Typical workflow
- Important options
- Efficient usage pattern
- Limitations
- Failure modes
- Related commands, APIs, plugins, and config
- Stability
- Source evidence
- Verification status

Do not treat a module as a feature unless a user can reach it.

### Configuration Inventory

For every config key:

- Canonical key
- Aliases
- Type
- Default
- Required or optional
- Allowed values or constraints
- Scope
- Reload behavior
- Secret status
- Environment equivalent
- CLI equivalent
- Example
- Source evidence
- Verification status

Never put real secrets in documentation.

Use obvious placeholders:

```text
YOUR_API_KEY
example.invalid
/path/to/project
```

### Environment Variable Inventory

For every environment variable:

- Name
- Purpose
- Type
- Default
- Required status
- Secret status
- Precedence
- Example
- Supported context
- Source evidence

Search direct environment reads and configuration framework bindings.

### API Inventory

For every API operation:

- Protocol
- Method or message type
- Path or operation name
- Stability
- Authentication
- Authorization
- Request fields
- Validation rules
- Response fields
- Error forms
- Pagination
- Streaming behavior
- Idempotency
- Rate or concurrency behavior when implemented
- Example request
- Example response
- Source evidence
- Verification status

Document actual wire behavior, not only handler intent.

### Plugin and Extension Inventory

For every plugin or extension surface:

- Name
- Purpose
- Discovery or registration method
- Lifecycle
- Required interface
- Capabilities
- Permissions
- Configuration
- Events or hooks
- Error handling
- Version compatibility
- Minimal example
- Production example
- Testing method
- Source evidence
- Stability

Clearly distinguish:

- Built-in plugins
- First-party optional plugins
- Third-party plugins
- Internal adapters
- Tool integrations
- Protocol integrations

### Tool Inventory

For agent harnesses and automation systems, document every tool with:

- Tool name
- Description
- Input schema
- Required fields
- Optional fields
- Output schema
- Failure schema
- Side effects
- Workspace boundaries
- Permission requirements
- Concurrency behavior
- Cancellation behavior
- Timeout behavior
- Examples
- Safety notes
- Source evidence

### Error Inventory

For important user-visible errors:

- Exact error or recognizable pattern
- Trigger
- Meaning
- Immediate recovery
- Root-cause fix
- Relevant logs
- Diagnostic command
- Related issue or limitation
- Source evidence

Group errors by user task rather than producing an unsearchable dump.

## Language and Framework Discovery Patterns

Use repository-specific evidence first. The following are discovery hints, not proof.

### Rust

Inspect:

- `clap`, `argh`, `structopt`, or custom command enums
- `serde` config structs and defaults
- `std::env` and environment wrappers
- `axum`, `actix-web`, `warp`, `rocket`, or custom routers
- Trait-based plugin or tool registries
- Cargo features
- Workspace members
- Examples and integration tests
- Error enums
- Build scripts

### TypeScript and JavaScript

Inspect:

- Commander, Yargs, Oclif, Clipanion, CAC, or custom parsers
- Zod, Valibot, Joi, TypeBox, JSON Schema, or handwritten validation
- Express, Fastify, Elysia, Hono, Next.js, or custom routers
- `process.env` access and config wrappers
- Plugin registration
- Package exports
- Scripts in package manifests
- Test fixtures and examples

### Go

Inspect:

- Cobra, urfave/cli, flag, or custom command trees
- Struct tags and config loaders
- `os.Getenv` and env helpers
- HTTP routers
- Interfaces and registration maps
- Build tags
- Examples and tests

### Python

Inspect:

- argparse, Click, Typer, Fire, or custom command dispatch
- Pydantic, dataclasses, settings classes, and environment loaders
- FastAPI, Flask, Django, or custom routes
- Entry points and console scripts
- Plugin discovery
- Type hints, tests, examples, and fixtures

For other languages, find equivalent command, schema, routing, registry, configuration, and test surfaces.

## Documentation Architecture

Do not create every file blindly. Adapt to repository size.

A complete repository may use:

```text
README.md
docs/
  index.md
  getting-started.md
  installation.md
  quickstart.md
  concepts/
    index.md
  guides/
    index.md
  commands/
    index.md
  features/
    index.md
  configuration/
    index.md
    environment-variables.md
  api/
    index.md
  plugins/
    index.md
  tools/
    index.md
  architecture/
    index.md
  operations/
    index.md
  troubleshooting.md
  security.md
  contributing.md
  reference/
    glossary.md
    compatibility.md
    exit-codes.md
  internal/
    documentation-coverage.md
    evidence-ledger.md
```

Small repositories should use fewer, denser files.

Large repositories should split by task or domain, not by arbitrary source directory.

## Required Core Documents

### README

The README should provide:

- One-sentence purpose
- Key capabilities
- Supported status
- Smallest successful quickstart
- Installation link
- Documentation map
- Important compatibility constraints
- Contribution link

Do not make the README a complete manual.

### Getting Started

A new user must be able to:

1. Understand prerequisites
2. Install the project
3. Configure the minimum required values
4. Run the project
5. Complete one meaningful task
6. Verify success
7. Know where to continue

### Commands

Commands should be organized by user goal.

Each command page should contain:

- Synopsis
- Purpose
- Prerequisites
- Arguments
- Options
- Behavior
- Examples
- Output
- Exit status
- Common mistakes
- Related commands
- Stability
- Version notes when required

### Features

Each feature page should explain:

- Why the feature exists
- When to use it
- Fastest useful workflow
- Detailed behavior
- Configuration
- Limitations
- Performance or concurrency considerations
- Failure recovery
- Related surfaces

### API

API docs should be contract-oriented.

Separate:

- Authentication
- Common types
- Errors
- Pagination
- Streaming
- Versioning
- Operations
- Examples
- Compatibility policy

### Plugins and Extensions

Plugin docs must make it possible to create, load, test, debug, and distribute a minimal plugin without reading implementation code.

### Architecture

Architecture docs should include:

- Major components
- Ownership boundaries
- Data and control flow
- Persistence
- Concurrency model
- Extension boundaries
- Security boundaries
- Failure boundaries
- Important design tradeoffs

Only include diagrams that improve understanding. Every diagram must match current code.

### Troubleshooting

Troubleshooting should be organized by symptom and task.

Use this pattern:

```markdown
## Command starts but never completes

**Symptoms**

**Likely causes**

**Check**

**Fix**

**Why this works**

**Related diagnostics**
```

## Efficient Usage Documentation

Do not stop at reference documentation.

For significant features, include an efficient-use section covering:

- Recommended workflow
- Correct order of operations
- How to avoid redundant work
- Concurrency recommendations
- Caching or reuse behavior
- Context or memory behavior
- Safe defaults
- When to use a simpler alternative
- Common performance mistakes
- Recovery from interrupted work

For an agent harness, explicitly document:

- How to choose execution mode
- When parallelism helps
- When chain execution is required
- Concurrency limits
- Subagent lifecycle
- Cancellation and resumption
- Context construction
- Memory scope
- Tool permissions
- Workspace confinement
- Review and verification loops

## Documentation Workflow

### Phase 1: Establish the Baseline

1. Read repository instructions.
2. Detect repository type and workspaces.
3. Inspect existing documentation.
4. Identify the current branch and dirty state.
5. Find the native build, test, lint, and docs commands.
6. Create the initial repository map.
7. Create documentation task ownership.

Do not make broad edits before the map exists.

### Phase 2: Parallel Discovery

Dispatch bounded discovery workers.

Each worker returns:

```yaml
domain: cli
status: complete
files_inspected:
  - src/cli/mod.rs
  - src/cli/agent.rs
surfaces:
  - id: CLI-agent-run
    classification: public-stable
    confidence: high
gaps:
  - "Exit code behavior is not covered by tests."
handoffs:
  - target_domain: configuration
    item: "CATCODE_MODEL overrides the command default."
recommended_documents:
  - docs/commands/agent-run.md
```

The coordinator merges results into canonical inventories.

### Phase 3: Coverage Plan

Create a coverage matrix before drafting:

| Surface | Discovered | Public | Documented | Verified | Owner |
|---|---:|---:|---:|---:|---|
| CLI commands | 42 | 39 | 31 | 28 | cli-docs |
| Config keys | 67 | 61 | 44 | 40 | config-docs |
| API operations | 18 | 18 | 12 | 11 | api-docs |
| Plugins | 7 | 5 | 3 | 3 | plugin-docs |

Prioritize:

1. Incorrect dangerous docs
2. Installation and quickstart blockers
3. Undocumented public commands
4. Undocumented configuration
5. APIs and plugins
6. Feature workflows
7. Troubleshooting
8. Architecture and internals
9. Polish

### Phase 4: Parallel Drafting

Assign each writer:

- Specific inventory IDs
- Owned files
- Required templates
- Required cross-links
- Required verification
- Prohibited areas

Writers must cite evidence internally while drafting.

Do not expose source citations in polished user docs unless the project wants them. Retain them in the internal evidence ledger.

### Phase 5: Verification

Run the strongest available checks:

- Build
- Unit tests
- Integration tests
- CLI help generation
- Example commands
- Config parsing
- API schema validation
- Link checking
- Markdown linting
- Code-block syntax checking
- Repository-native documentation build

When full execution is unsafe or impossible:

- Validate with parser definitions
- Use dry-run modes
- Use temporary directories
- Use test fixtures
- Mark unexecuted examples
- Record the limitation

Never run destructive production commands merely to verify docs.

### Phase 6: Cross-Document Review

Review for:

- Contradictory defaults
- Contradictory names
- Stale paths
- Broken links
- Duplicate content
- Missing prerequisites
- Unsupported promises
- Unmarked experimental behavior
- Missing security warnings
- Missing version constraints
- Terminology drift
- Examples that omit required setup
- Navigation dead ends

Assign a reviewer that did not author the document whenever practical.

### Phase 7: Gap Sweep

Re-run discovery after documentation is drafted.

Look for:

- Newly discovered commands
- Hidden aliases
- Undocumented config fields
- Unlinked feature pages
- Error variants absent from troubleshooting
- APIs present in routers but absent from docs
- Plugins registered but undocumented
- Examples inconsistent with current interfaces
- Deprecated behavior still presented as current

A documentation pass is not complete until the post-draft inventory is reconciled.

### Phase 8: Final Integration

The coordinator owns shared files:

- README
- Main docs index
- Navigation
- Glossary
- Coverage report
- Evidence ledger
- Final summary

Resolve terminology centrally.

Prefer one canonical explanation and link to it from other pages.

## Documentation Style

Use direct, technical language.

Prefer:

- Task-first headings
- Exact commands
- Concrete examples
- Tables for field reference
- Small focused code blocks
- Explicit defaults
- Explicit prerequisites
- Explicit limitations
- Cross-links to the next likely task

Avoid:

- Marketing language
- Vague claims
- “Simply” or “just” for complex operations
- Large unannotated code dumps
- Unexplained acronyms
- Fake certainty
- Repeated warnings
- Huge pages with unrelated audiences

Use the project's terminology exactly.

Define important terms once in a glossary.

## Source-of-Truth Precedence

When sources disagree, use this precedence:

1. Executed current behavior
2. Current public tests
3. Current runtime code
4. Current schemas and registries
5. Current checked-in examples
6. Current comments
7. Existing documentation
8. Changelog or historical documentation
9. Issues, plans, or TODOs

Record meaningful disagreements.

Do not silently choose the most convenient source.

## Security and Safety Documentation

Document security-relevant behavior when supported by evidence:

- Credential storage
- Secret precedence
- Workspace confinement
- Filesystem access
- Network access
- Plugin permissions
- Tool permissions
- Sandboxing
- Authentication
- Authorization
- Encryption
- Logging and redaction
- Remote execution
- Update trust
- Supply-chain boundaries

Never include:

- Real credentials
- Private endpoints
- Internal customer information
- Exploit instructions unrelated to safe project usage
- Sensitive operational data

## Versioning and Change Awareness

Determine the documentation target:

- Current working tree
- Current release
- Specific tag
- Specific branch
- Upcoming unreleased behavior

Do not mix versions without labels.

When unreleased behavior is documented, mark it clearly.

When a feature changed, add migration guidance if users could break existing workflows.

Use version qualifiers only where behavior differs.

## Existing Documentation Audit

Classify each existing document:

- `accurate`
- `partially-stale`
- `incorrect`
- `duplicate`
- `orphaned`
- `internal`
- `missing-critical-context`
- `replace`
- `remove`

For each stale claim, capture:

- Document path
- Claim
- Current evidence
- User impact
- Repair action

Do not delete historical or design documentation unless its purpose is understood.

## Quality Score

Score the final documentation from 0 to 100.

### Coverage: 30 points

- Commands: 6
- Configuration and environment: 6
- Features and workflows: 6
- APIs, plugins, and tools: 6
- Installation, operations, and troubleshooting: 6

### Accuracy: 25 points

- Claims trace to evidence
- Defaults match implementation
- Examples are validated
- Stability labels are correct
- No internal behavior is promised publicly

### Usability: 20 points

- New-user path succeeds
- Navigation is clear
- Task workflows are easy to find
- Efficient usage is explained
- Troubleshooting is actionable

### Consistency: 10 points

- Terminology
- Naming
- Defaults
- Cross-links
- Version statements

### Maintainability: 10 points

- Canonical inventories exist
- Evidence ledger exists
- Repeated content is minimized
- Ownership boundaries are clear
- Update workflow is documented

### Safety: 5 points

- Secrets are protected
- Dangerous operations are warned
- Permission boundaries are clear
- Destructive examples are safe

Completion gate:

- Overall score must be at least `95`
- No category may score below `90%` of its available points
- No known critical public surface may remain undocumented
- No high-impact claim may remain low confidence
- All verification failures must be fixed or explicitly reported

Do not declare completion below this gate.

## Required Final Report

Return a concise final report with:

```markdown
# Documentation Result

## Created
- ...

## Updated
- ...

## Removed or Consolidated
- ...

## Coverage
- Commands: 42/42
- Config keys: 61/61
- Environment variables: 14/14
- API operations: 18/18
- Plugins: 5/5
- User-visible features: 23/23

## Verification
- `cargo test`: passed
- `cargo run -- --help`: passed
- Documentation links: passed
- Executed examples: 31 passed, 2 marked illustrative

## Remaining Gaps
- None

## Quality Score
- 97/100
```

Report unresolved gaps honestly.

## Deliverable Rules

The repository should receive only artifacts that improve long-term use or maintenance.

Expected deliverables may include:

- Corrected README
- Structured documentation tree
- Complete command reference
- Complete config and environment reference
- Feature guides
- API reference
- Plugin or extension authoring guide
- Tool reference
- Architecture overview
- Troubleshooting guide
- Security guide
- Contributor guide
- Documentation coverage matrix
- Internal evidence ledger
- Documentation maintenance instructions

Do not add empty placeholder pages.

Do not add pages that merely say a feature exists.

Do not commit generated noise unless the repository intentionally tracks generated docs.

## Documentation Maintenance Instructions

When practical, add a short maintainer guide that explains:

- Which source files define public commands
- Which source files define config
- Which registries define plugins and tools
- Which tests verify examples
- Which documents must change with each public surface
- How to run documentation validation
- How to update the coverage inventory

The maintainer guide should make documentation drift harder.

## Stop Conditions

Stop only when:

- The requested scope is fully inventoried
- Public surfaces are classified
- Required documents exist
- Important examples are verified
- Cross-document review is complete
- Gap sweep is complete
- Quality gate passes
- Remaining limitations are reported

Do not stop because all workers returned.

Do not stop because the documentation build passes.

Do not stop because a large amount of text was created.

Stop when the repository's supported behavior is accurately and usefully documented.

## Activation Prompt

When this skill is invoked, follow this directive:

> Audit the repository, build evidence-backed inventories of every public surface in scope, and create or repair useful documentation. Use maximum safe concurrency for discovery and non-overlapping drafting. Do not invent behavior. Verify commands, examples, schemas, links, and workflows using repository-native tools. Keep repository facts project-scoped. Continue until the documentation quality gate passes or explicitly report the concrete blockers that prevent it.
