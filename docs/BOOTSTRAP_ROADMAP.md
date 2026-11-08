# Bootstrap Roadmap

## Phase 0 — Foundations

- establish repository conventions and reproducible local build
- [x] select the project license — GPL-3.0-or-later
- define capability identifiers and backend contracts
- define VFS handle and error model
- define transaction/journal model
- add baseline benchmarks for startup, memory, I/O, and package size

## Phase 1 — First usable core

- local directory browsing
- SAF integration
- copy / move / rename / delete primitives
- text viewer/editor foundation
- hex/binary viewer foundation
- basic archive read support
- basic APK structure and certificate inspection
- shell runner foundation

## Acceptance direction

The first usable build should prefer reliability over breadth.

New features should not enter the core unless they preserve:

- crash recovery expectations
- capability fallback behavior
- bounded memory use
- measurable performance
- minimal idle activity
