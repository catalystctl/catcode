<div align="center">
  <a href="https://github.com/catalystctl/catcode">
    <img src="docs/logo.svg" alt="Catalyst Code logo" width="120">
  </a>

# Catalyst Code

**A provider-independent coding agent for your terminal and browser.**

[![Latest Release](https://img.shields.io/github/v/release/catalystctl/catcode)](https://github.com/catalystctl/catcode/releases)
[![License](https://img.shields.io/github/license/catalystctl/catcode)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/catalystctl/catcode)](https://github.com/catalystctl/catcode/stargazers)
[![Website](https://img.shields.io/badge/website-code.catalystctl.com-orange)](https://code.catalystctl.com)

</div>

## What is Catalyst Code?

Catalyst Code, or **CatCode**, is an open-source coding-agent harness that runs on your own machine. Use it from a terminal or browser with cloud APIs, local models, self-hosted gateways, and OpenAI- or Anthropic-compatible providers.

Many coding agents lock you into one provider, require a hosted account, or hide how tools and agents operate. CatCode gives you control over the models, workspace, tools, approval rules, plugins, memory, and subagents used for each task. CatCode itself is self-hosted, and model requests are sent only to the provider endpoint you configure.

## Why CatCode?

* **Use different providers and models** without changing your coding workflow.
* **Approve destructive actions** before files or commands are changed.
* **Keep agents inside the selected workspace** with path-confinement protections.
* **Delegate work to subagents** for research, planning, implementation, and review.
* **Choose your interface** with both terminal and browser applications.
* **Extend the harness** with plugins, hooks, skills, custom tools, and provider integrations.
* **Recover and continue work** with persistent sessions, checkpoints, memory, and context compaction.

## Install

Prebuilt releases are available for Linux, macOS, and Windows. You do not need Rust, Go, or another compiler unless you are building CatCode from source.

Choose either the browser interface or the terminal interface.

### Browser interface

The browser interface is the recommended installation for most users.

It installs:

* The `catcode` terminal application
* The CatCode core
* The browser frontend
* A background service for the frontend

The browser frontend requires either [Node.js](https://nodejs.org/) or [Bun](https://bun.sh/) to run.

#### Linux and macOS

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
```

After installation, open:

```text
http://localhost:49283
```

To keep the frontend available only from the local machine:

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh \
  | bash -s -- --expose local
```

To use another port:

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh \
  | bash -s -- --port 8080
```

#### Windows

Open PowerShell and run:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1))) -WithWeb
```

After installation, open:

```text
http://localhost:49283
```

To install the frontend for local access only:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1))) -WithWeb -Expose local
```

### Terminal interface

Use this installation when you only want the terminal application.

#### Linux and macOS

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh \
  | bash -s -- --install
```

#### Windows

Open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1 | iex
```

Select the terminal installation from the installer menu.

After installation, open a new PowerShell window so the updated `PATH` is loaded.

> The agent's Bash tool on Windows requires Git Bash, WSL, or another Bash executable on `PATH`. Chat, file editing, search, and other native tools work without it.

### Homebrew

macOS users can install the terminal application from the CatCode Homebrew tap:

```bash
brew tap catalystctl/catcode
brew install catcode
```

Upgrade it later with:

```bash
brew upgrade catcode
```

### Standalone downloads

Installers, standalone executables, MSI packages, DMG images, and AppImages are also available from the [GitHub Releases](https://github.com/catalystctl/catcode/releases) page.

## Quick start

Open a terminal inside the project you want CatCode to work on:

```bash
cd /path/to/your/project
catcode
```

Once CatCode opens:

1. Run `/login` and select a provider.
2. Run `/model` and select a model.
3. Describe what you want the agent to do.

Example:

```text
Review this project, explain how it is structured, and identify the most important bugs to fix.
```

CatCode uses the directory where it was launched as the workspace. File operations are confined to that workspace unless you explicitly change the configuration.

## Essential commands

| Command     | Description                                       |
| ----------- | ------------------------------------------------- |
| `/login`    | Add or select a model provider                    |
| `/model`    | View and switch models                            |
| `/approval` | Configure when tool actions require approval      |
| `/settings` | Open CatCode settings                             |
| `/goal`     | Start a larger planned task using multiple agents |
| `/help`     | Display all available commands                    |

You can also run shell commands directly from the terminal interface:

```text
!git status
```

Use two exclamation marks when you do not want the command output added to the model context:

```text
!!clear
```

## Providers

CatCode includes provider presets and supports additional providers through plugins.

Built-in presets currently include:

* Umans
* OpenCode Go
* OpenRouter

You can also connect compatible local servers, gateways, and custom endpoints. Use `/login` inside CatCode to configure a provider.

Environment variables may be used for API keys so secrets do not need to be entered into the interface.

## Safety

CatCode provides several layers of protection while an agent works:

* Workspace path confinement
* Approval prompts for destructive tools
* Restricted-path protection
* Filesystem checkpoints
* Optional operating-system sandboxing
* Tool and session logs
* Configurable network access

The default approval mode asks before destructive operations. Review commands and file changes before approving them, especially when working in an important repository.

## Updating

Update the terminal application with:

```bash
catcode --update
```

When the browser frontend was installed through the CatCode installer, the update command also refreshes its installed components and restarts the service.

Updates can also be started from:

```text
Settings → About → Update CLI + frontend
```

## Uninstall

### Linux and macOS

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh \
  | bash -s -- --uninstall
```

### Windows

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1))) -Uninstall
```

### Homebrew

```bash
brew uninstall catcode
```

## Documentation

More detailed guides are available in the [CatCode documentation](docs/index.md), including:

* Installation and configuration
* Commands and keyboard controls
* Model providers
* Tools and approval rules
* Subagents and goal mode
* Memory and skills
* Plugins and custom tools
* Browser frontend deployment
* Architecture and development

## Contributing

Contributions, bug reports, and feature requests are welcome.

* [Report a bug](https://github.com/catalystctl/catcode/issues/new?labels=bug)
* [Request a feature](https://github.com/catalystctl/catcode/issues/new?labels=enhancement)
* [View open issues](https://github.com/catalystctl/catcode/issues)

Development and build instructions are available in the [contributor documentation](docs/index.md).

## License

Catalyst Code is distributed under the [MIT License](LICENSE).
