# SoliDB Project Overview

This file serves as a persistent knowledge base for Gemini to understand the SoliDB project architecture, features, and roadmap.

## Project Summary
SoliDB is a high-performance, ACID-compliant, distributed database written in **Rust**. It features a custom query language (SDBQL), integrated Lua scripting, and a real-time web-based management interface.

## Core Architecture
- **Language**: Rust
- **Storage**: Custom KV-based storage with ACID transaction support.
- **Client Protocol**: MessagePack-based communication.
- **Web Interface**:
  - Backend: Rust with `etlua` templates.
  - Frontend: Riot.js for reactive components. Riot.js components are compiled automatically by the browser and do not require a build step.
  - Routing: Custom router in `talks-app.riot`.
- **Real-time**: WebSockets for Live Queries and monitoring.

## Talks Application
- **Concept**: A mini Slack-like communication platform built on top of SoliDB.
- **Features**: Real-time channels, messaging, threading, and huddle/video calls.
- **Implementation**: Uses Riot.js for the UI (components are `talks-*.riot` files) and WebSockets for real-time updates. Located at `www/app/views/talks/`.

## SDBQL (SoliDB Query Language)
- Supports standard CRUD operations.
- Advanced operators: `LIKE`, `NOT LIKE`, `=~` (RegEx), `!~` (Not RegEx).
- Built-in functions:
  - Numeric: `SQRT`, `POW`, `ABS`, etc.
  - Hashing: `MD5`, `SHA256`.
  - Utility: `SLEEP`, `ASSERT`.
- Transactional support via `BEGIN`, `COMMIT`, `ROLLBACK`.

## Lua Scripting
- Sandboxed Lua environment for server-side logic.
- Access to database collections and logging via a dedicated `_logs` collection.
- Uses MessagePack for data exchange between Rust and Lua.

## Key Directories
- `src/`: Core Rust implementation.
  - `src/sdbql/`: Parser and executor for SDBQL.
  - `src/scripting/`: Lua environment integration.
  - `src/server/`: HTTP and WebSocket server routes.
- `www/`: Web management interface.
  - `www/app/views/`: Etlua templates.
  - `www/static/`: Frontend assets (Riot components, JS, CSS).
- `clients/`: Client libraries for various languages (Rust, PHP, Ruby, Python, Go, Elixir, Bun).
- `docs/`: Project documentation.

## Recent Updates
- Improved SDBQL function documentation.
- Refined multi-core parallel insert benchmarks.
- Enhanced real-time monitoring dashboard with WebSocket updates.
- Implemented ACID transaction support and documentation.
