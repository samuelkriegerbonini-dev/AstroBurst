# AstroBurst Flatpak

## Pre-requisitos

```bash
sudo apt install flatpak flatpak-builder
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install flathub org.freedesktop.Platform//24.08 org.freedesktop.Sdk//24.08
```

## Estrutura

```
flatpak/
  com.astroburst.desktop.yml          # manifesto Flatpak
  com.astroburst.desktop.desktop      # desktop entry
  com.astroburst.desktop.metainfo.xml # AppStream metainfo (Flathub)
  icons/
    icon.svg                          # copiar de src/assets/logo.svg
    icon-128.png                      # gerar: convert logo.svg -resize 128x128 icon-128.png
    icon-256.png
    icon-512.png
```

## Preparar o tarball

Antes de buildar o Flatpak, crie o release tarball:

```bash
cd AstroBurst

# Build release
cargo tauri build --bundles none

# Criar tarball
cd src-tauri/target/release
tar czf astroburst-0.4.6-linux-x86_64.tar.gz \
  astroburst \
  ../../../src-tauri/resources/ \
  ../release/bundle/

# Calcular SHA256
sha256sum astroburst-0.4.6-linux-x86_64.tar.gz
```

Substitua `REPLACE_WITH_ACTUAL_SHA256` no manifesto pelo hash real.

## Build local

```bash
cd flatpak/

# Build
flatpak-builder --force-clean build-dir com.astroburst.desktop.yml

# Instalar local
flatpak-builder --user --install --force-clean build-dir com.astroburst.desktop.yml

# Rodar
flatpak run com.astroburst.desktop
```

## Validar metainfo

```bash
# Instalar appstream-util
sudo apt install appstream-util

# Validar
appstream-util validate com.astroburst.desktop.metainfo.xml

# Testar desktop entry
desktop-file-validate com.astroburst.desktop.desktop
```

## Publicar no Flathub

1. Fork https://github.com/flathub/flathub

2. Criar branch com o app ID:
```bash
git checkout -b com.astroburst.desktop
```

3. Copiar os arquivos:
```bash
cp com.astroburst.desktop.yml .
cp com.astroburst.desktop.metainfo.xml .
```

4. Commit e PR:
```bash
git add -A
git commit -m "Add com.astroburst.desktop"
git push origin com.astroburst.desktop
```

5. Abrir PR em https://github.com/flathub/flathub/pulls

6. Review do Flathub leva 1-2 semanas. Checklist:
   - Metainfo valida (appstream-util validate)
   - Desktop entry valida (desktop-file-validate)
   - Screenshots acessiveis via HTTPS
   - License correta (GPL-3.0-only)
   - Sem --filesystem=host (usamos home:ro)

## Atualizar versao

Para cada release:
1. Atualizar URL e sha256 no manifesto
2. Adicionar novo `<release>` no metainfo.xml
3. PR no repo Flathub do app (apos aprovacao inicial, o repo fica em https://github.com/flathub/com.astroburst.desktop)

## Notas

- WebKit2GTK: o runtime 24.08 inclui webkit2gtk-4.1 necessario para Tauri 2
- WebGPU: `--device=dri` permite acesso ao GPU para Vulkan/WebGPU
- Network: necessario para plate solve via astrometry.net API
- O Flatpak sandbox isola o app; `--filesystem=home:ro` permite ler FITS do usuario
- `--filesystem=xdg-documents` e `xdg-download` permitem salvar exports
