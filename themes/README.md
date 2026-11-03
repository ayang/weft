# Weft themes

The site frontend lives outside the Rust code.

```text
themes/default/
├── templates/   Tera HTML templates (`*.html`)
└── static/      CSS, JS, ECharts, images and other public assets
```

The selected theme is read from `SITE_THEME`:

```bash
SITE_THEME=default
```

## Create a theme

1. Copy the default theme:

   ```bash
   cp -a themes/default themes/mytheme
   ```

2. Edit `themes/mytheme/templates/` and `themes/mytheme/static/`.
3. Start with:

   ```bash
   SITE_THEME=mytheme ./scripts/start-dev.sh
   ```

The server must be restarted after changing `SITE_THEME`; template files (not
the theme path) hot-reload in development. A theme can override any public or
admin template. Keep the `/static/...` URLs unchanged so assets resolve to
`themes/<theme>/static`.

Do not put uploads in the theme directory. Runtime media is stored under
`media/uploads/` (configurable with `MEDIA_DIR`) and served from `/media/...`.
