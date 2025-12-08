# SoliDB Web Dashboard

The official web-based administration interface for SoliDB.

## Overview

This dashboard provides a graphical user interface to manage your SoliDB instance. It allows you to:

- **Monitor Cluster Status**: View the health and status of your SoliDB nodes.
- **Manage Databases & Collections**: Create, delete, and configure databases and collections.
- **Data Explorer**: View, edit, and delete documents.
- **AQL Query Editor**: Run AQL queries with syntax highlighting and view results.
- **Visualizations**: View charts and statistics about your data.

## Technology Stack

The dashboard is built using:

- **[LuaOnBeans](https://luaonbeans.org)**: A full-stack Lua framework running on Redbean.
- **[Riot.js](https://riot.js.org)**: A simple and elegant component-based UI library.
- **[Tailwind CSS](https://tailwindcss.com)**: A utility-first CSS framework (v4).

## Prerequisites

To develop or build the dashboard, you need:

- **Node.js & npm**: For managing frontend dependencies.
- **LuaOnBeans CLI**: For running the development server.

```bash
npm install -g luaonbeans-cli
```

## Setup & Development

1.  **Install Dependencies**:
    ```bash
    cd www
    npm install
    ```

2.  **Run Development Server**:
    ```bash
    beans dev
    ```
    This will start the dashboard on `http://localhost:3000` (default port, check console output).
    It expects a generic SoliDB instance running on `http://localhost:6745`.

3.  **Build for Production**:
    The production build is handled by the `luaonbeans` build process.
    ```bash
    beans build
    ```

## Project Structure

- `app/views`: Server-side views (Etlua templates).
- `app/components`: Riot.js frontend components.
- `app/controllers`: Lua controllers handling backend logic.
- `public`: Static assets (compiled CSS, JS, images).
- `config`: Configuration files.
