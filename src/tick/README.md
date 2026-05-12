# Tick Adapter Layer

`omb/src/tick` is now a thin compatibility layer for the shared deterministic systems in `omoba-core/src/runtime/native/tick`.

The backend dispatcher still expects the local `crate::comp::ecs::System` wrapper, so each module exposes a `Sys` adapter around the matching core system. Gameplay logic should be changed in `omoba-core`, not copied back into this directory.
