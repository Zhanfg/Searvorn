# Architecture Seed

This document defines only the initial boundaries.

```text
Android UI
   │
   ▼
Application Layer
   │
   ├── Workspace
   ├── Task / transaction control
   └── Capability router
   │
   ▼
Core
   ├── VFS
   ├── I/O
   ├── Text / binary
   ├── Archive
   └── Android package inspection
   │
   ▼
Backends
   ├── App sandbox
   ├── SAF / MediaStore
   ├── Shizuku
   ├── System privilege
   └── Root / kernel capability
```

## Rules

### Capability routing

Features ask for concrete capabilities, not for a user class such as "root user".

Example capability families:

- filesystem read/write
- package query/install
- process execution
- Android shell access
- mount namespace access
- raw device access

A backend may satisfy one or more capabilities. The router chooses the best available backend for each operation.

### VFS

Editors and analyzers must not care whether content came from:

- direct filesystem access
- SAF
- privileged backend
- archive entry
- future remote storage

The VFS API should expose handles/streams rather than forcing complete copies into memory.

### Data safety

Destructive operations should move toward:

```text
read source
  -> prepare transaction
  -> validate
  -> commit
  -> verify
```

The design must keep recovery possible when the process is killed, storage fills, or permissions disappear during an operation.

### Performance

Optimization priorities:

1. avoid unnecessary work
2. avoid unnecessary copies and allocations
3. use compact data structures
4. parallelize only when measurement shows a benefit
5. specialize hot paths only after a correct reference path exists

Idle background work should approach zero.
