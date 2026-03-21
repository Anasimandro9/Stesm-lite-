# 🎮 Steam Lite

Cliente Steam ligero para PCs con pocos recursos (~50MB RAM vs 400MB del oficial).

## Cómo bajarte el .exe sin instalar nada

1. Andá a la pestaña **Actions** de este repositorio
2. Clic en el último workflow "Build SteamLite"
3. Abajo del todo en **Artifacts** → descargá **SteamLite**
4. Extraé el ZIP y ejecutá `steamlite.exe`

## Cómo subir el código a GitHub (primera vez)

1. Creá una cuenta en github.com
2. Creá un repositorio nuevo (nombre: `steamlite`, privado o público)
3. En tu PC abrí CMD y ejecutá:

```
git init
git add .
git commit -m "primer commit"
git remote add origin https://github.com/TU_USUARIO/steamlite.git
git push -u origin main
```

GitHub Actions compila solo al subir. En ~5 minutos tenés el .exe listo para bajar.
