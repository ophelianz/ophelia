# Ophelia Core Diagrams

These diagrams are a sanity check. If a diagram gets too tangled, the code probably will too.

## Current Package Shape

```mermaid
graph TD
  Workspace["root workspace"] --> CorePackage["crates/core package ophelia"]
  Workspace --> GuiSource["crates/ophelia-gui source parked"]
  CorePackage --> Engine["engine"]
  CorePackage --> State["state and DB worker"]
  CorePackage --> Http["HTTP range engine"]
  CorePackage --> Config["CoreConfig and CorePaths"]
  GuiSource -. "not in workspace yet" .-> Adapter["future GUI adapter"]
  Adapter -. "will pass plain config" .-> CorePackage
```

## Target Crate Shape

```mermaid
graph TD
  Gui["ophelia-gui"] --> Core["ophelia crate"]
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
  Url["URL request"] --> Spec["DownloadSpec from CoreConfig"]
  Spec --> EngineAdd["DownloadEngine::add"]
  EngineAdd --> Controller["EngineController"]
  Controller --> Seed["EngineEvent::TransferAdded"]
  Controller --> Provider["provider::spawn_task"]
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
  Progress --> Controller
  Controller --> Events["EngineEvent stream"]
  Seed --> Events
  Events --> Frontend["GUI or CLI adapter"]
  Frontend --> Rows["rows, progress, history, stats graph"]
  Runner --> Finalize["finalize_part_file"]
  Single --> Finalize
  Finalize --> Disk["file on disk"]
```

The current download path label still mentions modal and IPC because that is where the old GUI sends requests from. In this slice, those files are parked outside the workspace while core gets cleaned up.

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

  subgraph Core["ophelia crate"]
    CoreApi["async engine API"]
    Controller["engine task"]
    Providers["providers"]
    Store["state store"]
    DiskWriter["disk writer"]
  end

  Gui --> SettingsToCore --> CoreApi
  Cli --> ArgsToRequest --> CoreApi
  CoreApi --> Controller
  Controller --> Providers
  Controller --> Store
  Providers --> DiskWriter
  Controller --> EngineEvents["EngineEvent stream"]
  EngineEvents --> EventsToRows
  EventsToRows --> Gui
  EventsToRows --> Cli
```

## Runtime Ownership

```mermaid
flowchart TD
  CurrentCaller["caller-owned Tokio runtime"] --> Handle["tokio runtime Handle"]
  Handle --> CurrentEngine["DownloadEngine::spawn_on"]
  CurrentEngine --> CurrentController["spawns EngineController"]

  TargetGui["target GUI"] --> GuiRuntime["GUI-owned runtime bridge"]
  TargetCli["target CLI"] --> CliRuntime["tokio::main"]
  GuiRuntime --> Handle
  CliRuntime --> Handle
  CurrentEngine --> AsyncCore["core async output"]
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
    Job["RangeWriteJob"]
    Writer["RangeDiskWriter on spawn_blocking"]
    Event["WorkerEvent"]
  end

  Balanced --> Pending
  Sequential --> Pending
  Pending --> Active
  Active --> Worker
  Worker --> Job
  Job --> Writer
  Writer --> Worker
  Worker --> Event
  Event --> Completed
  Event --> Hedges
```

## Range Disk Path

```mermaid
flowchart TD
  Worker["range worker downloads bytes"] --> Job["write job with offset and bytes"]
  Job --> Queue{"bounded write queue"}
  Queue --> Writer["RangeDiskWriter owns file"]
  Writer --> Result{"write result"}
  Result -->|"ok"| WorkerConfirm["worker receives confirmation"]
  Result -->|"worker gone"| Orphan["writer returns orphaned result on shutdown"]
  Result -->|"error"| Fail["WorkerFailure"]
  WorkerConfirm --> Count["WorkerEvent::BytesWritten"]
  Orphan --> Count
  Count --> Progress["progress and write stats"]
  Progress --> Events["EngineEvent"]
  Queue -->|"pause requested"| Drain["stop workers and drain accepted jobs"]
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
  Controller["engine controller"] --> Writer
  Restore --> Controller
  History --> GuiHistory["GUI history rows"]
  History --> CliHistory["future CLI history output"]
```

## Core Event Stream

```mermaid
flowchart TD
  Command["EngineCommand"] --> Controller["engine task"]
  Controller --> Task["provider task"]
  Task --> Runtime["TaskRuntimeUpdate"]
  Runtime -->|"progress tick"| Progress["EngineEvent::Progress"]
  Runtime -->|"bytes written"| WriteStats["EngineEvent::DownloadBytesWritten"]
  Runtime -->|"destination changed"| Destination["EngineEvent::DestinationChanged"]
  Runtime -->|"control support changed"| Support["EngineEvent::ControlSupportChanged"]
  Runtime -->|"chunk map changed"| ChunkMap["EngineEvent::ChunkMapChanged"]
  Runtime -->|"done"| Done["finish DB state and start next queued task"]
  Controller --> Seed["EngineEvent::TransferAdded or TransferRestored"]
  Controller --> Removal["EngineEvent::LiveTransferRemoved"]
  WriteStats --> Frontend["frontend adapter"]
  Progress --> Frontend
  Destination --> Frontend
  Support --> Frontend
  ChunkMap --> Frontend
  Seed --> Frontend
  Removal --> Frontend
  Done --> Controller
  Controller --> Reply
```
