# Repository Guidelines

## Project Structure & Module Organization
- Root scripts target the Vite + React frontend (`src/`), the Tauri Rust host (`src-tauri/`), and a Go backend (`server/`).
- Frontend entrypoints: `src/main.tsx` mounts the app, `src/App.tsx` holds the UI; shared helpers sit in `src/lib/`, static assets in `src/assets/` and `public/`.
- Tauri config is in `src-tauri/tauri.conf.json`; Rust sources live in `src-tauri/src/` (`main.rs` entry, `lib.rs` support). Native builds land in `src-tauri/target/`.
- The Go service uses `server/cmd/` for binaries and `server/internal/` for packages; dependencies are tracked in `server/go.mod`.
- Build outputs are written to `dist/` (web) and `src-tauri/target/` (desktop); avoid editing generated files.

## Build, Test, and Development Commands
- Install deps: `pnpm install` (lockfile is `pnpm-lock.yaml`).
- Web dev server: `pnpm dev` (Vite on localhost); desktop dev: `pnpm tauri dev` (Vite + Tauri shell).
- Production build: `pnpm build` (TypeScript check then Vite build); preview with `pnpm preview`.
- Android: `pnpm android:dev` for live dev, `pnpm android:apk` for release APK.
- Go service: from `server/`, run `go test ./...` and `go build ./...`.
- Rust host: run `cargo fmt` / `cargo clippy` in `src-tauri/` before shipping native changes.

## Coding Style & Naming Conventions
- TypeScript/React: use functional components and hooks; prefer explicit types over `any`; keep 2-space indentation. Name components in `PascalCase` and helpers in `camelCase`.
- Styling: keep global tweaks in `src/App.css`; colocate component-specific styles when practical.
- Rust: follow `rustfmt` defaults; module names snake_case, types in `PascalCase`.
- Go: run `gofmt`; packages snake_case, exported identifiers `PascalCase`.
- Import paths: favor relative paths within `src/`; keep shared utilities in `src/lib/` to avoid deep relative chains.

## Testing Guidelines
- No frontend harness is configured; when adding tests, prefer `vitest` + `@testing-library/react` and colocate specs (`Component.test.tsx`).
- Go: table-driven tests named `TestXxx`; ensure packages pass `go test ./...`.
- Manual QA: confirm primary flows (launch, navigation, file access) in web and Tauri shells; verify Android builds start after native changes.

## Commit & Pull Request Guidelines
- History is minimal; adopt Conventional Commits (`feat:`, `fix:`, `chore:`, `refactor:`, `docs:`) for clarity.
- Each PR should state scope, testing performed, and user-visible changes; attach screenshots for UI adjustments and note platform (web/desktop/android) covered.
- Keep changes scoped: separate frontend, Rust host, and Go server adjustments when possible. Document config changes (`tauri.conf.json`, Android signing, env vars) in the PR body.
