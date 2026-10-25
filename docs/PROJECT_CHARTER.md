# Project Charter

## Status

Early design and bootstrap stage.

## Initial goal

Searvorn is an open-source, local-first system file manager and workbench for Android.

The first implementation should focus on a small, reliable core before expanding scope.

## Principles

- Local-first. Core workflows must not depend on a remote service.
- Open and auditable. Source, build logic, and release provenance should be reviewable.
- Capability-based. Normal Android permissions, SAF, Shizuku, system privileges, Root, and kernel-level capabilities extend the same product instead of creating separate editions.
- No artificial feature reduction. Lack of elevated privilege only limits operations that truly require that privilege.
- Correctness first. Performance work must not weaken data integrity.
- Transactional changes for destructive operations where practical.
- Streaming and incremental processing by default; avoid whole-file loading without a reason.
- Small runtime surface, low idle cost, low memory traffic, and minimal package size.
- Shared core abstractions instead of duplicated parsers, VFS implementations, indexes, or diff engines.

## Initial scope

The bootstrap phase covers:

1. Unified VFS and capability routing.
2. Local file operations and Android storage integration.
3. Text and binary inspection/editing foundations.
4. Archive foundations.
5. APK inspection foundations.
6. Shell execution foundations with host access constrained by the available capability set.

APK/DEX/resource reverse engineering, Linux runtime, build tooling, security analysis, plugins, and MCP are later layers. They are design targets, not claims about the current implementation.

## Licensing

License selection is pending. A release must not be presented as fully open-source until an explicit license is committed.
