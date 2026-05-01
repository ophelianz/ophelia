# Ophelia Core Diagrams

These diagrams are a sanity check. If a diagram gets too tangled, the code probably will too.

## Current Package Shape

```mermaid
graph TD
  Main["src/main.rs starts GPUI"] --> App["src/app.rs Downloads"]
  App --> Engine["src/engine DownloadEngine"]
  App --> Settings["src/settings Settings"]
  App --> Ipc["src/ipc browser IPC"]
  App --> Views["src/views and src/ui"]
  App --> State["src/engine/state DB worker"]
  Engine --> Settings
  Engine --> State
  Engine --> Platform["src/platform paths"]
  State --> Platform
  Ipc --> Request["engine AddDownloadRequest"]
  Request --> Engine
```

## Target Crate Shape

```mermaid
graph TD
  Gui["ophelia-gui"] --> Core["ophelia-core"]
  Cli["ophelia-cli"] --> Core
  Gui --> GuiSettings["GUI settings and GPUI state"]
  GuiSettings --> CoreConfig["CoreConfig and CorePaths"]
  Cli --> CliArgs["URL and output path"]
  CliArgs --> DownloadRequest["DownloadRequest"]
  CoreConfig --> Core
  DownloadRequest --> Core
  Core --> EngineEvents["EngineEvent stream"]
  EngineEvents --> GuiRows["GUI rows and graphs"]
  EngineEvents --> CliOutput["CLI progress and exit code"]
```

## Current Download Path

```mermaid
flowchart TD
  Url["URL from modal or IPC"] --> Spec["DownloadSpec from Settings"]
  Spec --> EngineAdd["DownloadEngine::add"]
  EngineAdd --> Actor["EngineActor"]
  Actor --> Provider["provider::spawn_task"]
  Provider --> Task["http::download_task"]
  Task --> Probe["probe server"]
  Probe --> RangeQuestion{"range support and known size?"}
  RangeQuestion -->|"yes"| Plan["build range plan"]
  RangeQuestion -->|"no"| Single["single_download"]
  Plan --> Runner["run_range_download"]
  Runner --> Scheduler["RangeScheduler"]
  Scheduler --> Attempt["start range attempt"]
  Attempt --> Worker["run_range_worker"]
  Worker --> Event["WorkerEvent"]
  Event --> Scheduler
  Scheduler --> Progress["ProgressUpdate and runtime update"]
  Progress --> Actor
  Actor --> App["Downloads polling loop"]
  App --> Rows["transfer rows, history, stats graph"]
  Runner --> Finalize["finalize_part_file"]
  Single --> Finalize
  Finalize --> Disk["file on disk"]
```

## Target Core Boundary

```mermaid
flowchart LR
  subgraph Frontends["frontends"]
    Gui["GUI"]
    Cli["CLI"]
  end

  subgraph Adapter["small adapters"]
    SettingsToCore["Settings to CoreConfig and CorePaths"]
    ArgsToRequest["CLI args to DownloadRequest"]
    EventsToRows["EngineEvent to rows or terminal output"]
  end

  subgraph Core["ophelia-core"]
    CoreApi["async engine API"]
    Actor["engine task"]
    Providers["providers"]
    Store["state store"]
    DiskWriter["disk writer"]
  end

  Gui --> SettingsToCore --> CoreApi
  Cli --> ArgsToRequest --> CoreApi
  CoreApi --> Actor
  Actor --> Providers
  Actor --> Store
  Providers --> DiskWriter
  Actor --> EventsToRows
  EventsToRows --> Gui
  EventsToRows --> Cli
```

## Runtime Ownership

```mermaid
flowchart TD
  CurrentGui["current GUI"] --> CurrentEngine["DownloadEngine::new"]
  CurrentEngine --> HiddenRuntime["creates Tokio runtime"]
  HiddenRuntime --> CurrentActor["spawns EngineActor"]

  TargetGui["target GUI"] --> GuiRuntime["GUI-owned runtime bridge"]
  TargetCli["target CLI"] --> CliRuntime["tokio::main"]
  GuiRuntime --> AsyncCore["core async API"]
  CliRuntime --> AsyncCore
  AsyncCore --> CoreTasks["core tasks"]
  CoreTasks --> Shutdown["cancel and wait where cleanup matters"]
```

## Range Engine Today

```mermaid
flowchart LR
  subgraph Ordering["range ordering"]
    Balanced["Balanced makes small work units"]
    Sequential["Sequential uses one request"]
    Pending["pending ranges"]
  end

  subgraph Scheduler["scheduler"]
    Active["active attempts"]
    Completed["completed RangeSet"]
    Hedges["hedge groups when enabled"]
  end

  subgraph Workers["workers"]
    Worker["range worker"]
    Write["std file write_at"]
    Event["WorkerEvent"]
  end

  Balanced --> Pending
  Sequential --> Pending
  Pending --> Active
  Active --> Worker
  Worker --> Write
  Write --> Event
  Event --> Completed
  Event --> Hedges
```

## Target Range Disk Path

```mermaid
flowchart TD
  Worker["range worker downloads bytes"] --> Job["write job with offset and bytes"]
  Job --> Queue{"bounded write queue"}
  Queue --> Writer["one disk writer owns file"]
  Writer --> Result{"write result"}
  Result -->|"ok"| Count["count bytes as written"]
  Result -->|"error"| Fail["fail download"]
  Count --> Progress["progress and write stats"]
  Progress --> Events["EngineEvent"]
  Queue -->|"pause requested"| Drain["drain or safely stop writer"]
  Drain --> Snapshot["save reusable ranges"]
```

## Persistence Ownership

```mermaid
graph TD
  CorePaths["CorePaths"] --> DbPath["database path"]
  DbPath --> Store["state store"]
  Store --> Schema["schema and migrations"]
  Store --> Writer["DB writer"]
  Store --> Restore["restore loader"]
  Store --> History["history reader"]
  Actor["engine actor"] --> Writer
  Restore --> Actor
  History --> GuiHistory["GUI history rows"]
  History --> CliHistory["future CLI history output"]
```

## Core Event Stream

```mermaid
flowchart TD
  Command["EngineCommand"] --> Actor["engine task"]
  Actor --> Task["provider task"]
  Task --> Event{"what happened?"}
  Event -->|"bytes written"| WriteStats["TransferWriteStats"]
  Event -->|"progress tick"| Progress["TransferProgress"]
  Event -->|"pause saved"| Paused["TransferPaused"]
  Event -->|"rename done"| Finished["TransferFinished"]
  Event -->|"error"| Failed["TransferFailed"]
  Event -->|"chunk map dirty"| ChunkMap["TransferChunkMapChanged"]
  WriteStats --> Frontend["frontend adapter"]
  Progress --> Frontend
  Paused --> Frontend
  Finished --> Frontend
  Failed --> Frontend
  ChunkMap --> Frontend
```
