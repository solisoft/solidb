# admin

A Soli MVC application.

## Getting Started

### Development Server

Start the development server with hot reload:

```bash
soli serve . --dev
```

Your app will be available at [http://localhost:5011](http://localhost:5011)

### Production Server

Start the production server:

```bash
soli serve . --port 5011
```

Or run as a daemon:

```bash
soli serve . -d
```

## Project Structure

```
admin/
├── app/
│   ├── assets/
│   │   └── css/
│   │       └── application.css  # Source CSS with Tailwind directives
│   ├── controllers/     # Request handlers
│   ├── models/          # Data models
│   └── views/           # HTML templates
│       ├── home/        # Home page views
│       └── layouts/     # Layout templates
├── config/
│   └── routes.sl      # Route definitions
├── db/
│   └── migrations/      # Database migrations
├── public/              # Static assets (compiled output)
│   ├── css/
│   │   └── application.css  # Compiled CSS (generated)
│   ├── js/
│   └── images/
├── tests/               # Test files
├── package.json         # npm dependencies
└── tailwind.config.js   # Tailwind configuration
```

## Database Migrations

Generate a new migration:

```bash
soli db:migrate generate create_users
```

Run pending migrations:

```bash
soli db:migrate up
```

Rollback last migration:

```bash
soli db:migrate down
```

Check migration status:

```bash
soli db:migrate status
```

## Documentation

- [Soli MVC Documentation](https://soli.solisoft.net/docs)
- [Soli Language Reference](https://soli.solisoft.net/docs/soli-language)
- [Tailwind CSS](https://tailwindcss.com/docs)

## License

MIT
