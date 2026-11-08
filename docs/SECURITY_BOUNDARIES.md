# Security boundaries

This note records constraints that are easy to lose while the storage layer is still small.

## Local filesystem backend

`LocalFsBackend` scopes paths to a configured root and rejects lexical parent traversal. Existing
paths are canonicalized before they are opened, so a normal symlink cannot be used to point an open
outside that root.

That is useful application-level confinement. It is **not** a privilege boundary against an
attacker who can concurrently mutate the same directory tree. Path validation followed by a
separate open still has a time-of-check/time-of-use window.

Do not reuse this implementation unchanged for a Root or system-privileged backend.

## Privileged filesystem backend

A privileged backend should resolve paths relative to already-open directory descriptors. On Linux
kernels that provide it, prefer `openat2` with constraints such as `RESOLVE_BENEATH` and
`RESOLVE_NO_MAGICLINKS`. A fallback should walk components with fd-relative operations and
`O_NOFOLLOW` rather than canonicalizing a string path and opening it later.

The privileged backend should also keep these rules:

- least privilege is selected per operation, not per user session;
- raw block access is a separate capability from ordinary filesystem access;
- destructive operations use a staged/transactional path when the target filesystem permits it;
- crossing mount points or namespaces must be an explicit operation, never an accidental side effect.

The Android app sandbox, SAF, Shizuku, system privilege, and Root backends should all feed the same
VFS contracts. Their trust boundaries are different, so their implementations do not need to be.
