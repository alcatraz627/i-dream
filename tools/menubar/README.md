# tools/menubar: the old widget

This is the first i-dream menu bar widget. It has been replaced by
`tools/widget2`, which is the one that runs on this machine.

Do not build this one. It uses the same app name as the new widget
(`i-dream-bar`), so building it stops the live widget and puts the old one in
its place. `build.sh` now exits with an error and does nothing else.

To build, install, check or remove the live widget:

```bash
i-dream widget install     # build and start at login
i-dream widget status      # what is built, installed and running
i-dream widget logs        # tail the widget log
i-dream widget uninstall   # stop it and remove it
```

The same steps exist as scripts in `tools/widget2/` (`build.sh`, `install.sh`).

The sources here are kept for reference. The Swift check in
`.github/workflows/ci.yml` still compiles them directly, without `build.sh`.
