<p align="center">
  <img src="assets/icon.svg" width="96" alt="">
</p>

<h1 align="center">ahri</h1>

<p align="center">Print e gravação de tela para KDE Plasma no Wayland.<br>Um daemon em Rust que abre na hora e copia direto para o clipboard.</p>

<br>

## Print

**Win+Shift+S** congela a tela. Arraste uma área, desenhe ou aponte, e aperte **Ctrl+C**.

![Print com traço e seta](docs/print.png)

| Tecla | Ação |
| --- | --- |
| `1` / `D` | Desenhar |
| `2` / `A` | Seta |
| `3` | Abrir cores, depois `1`–`6` para escolher |
| `C` | Próxima cor |
| `Ctrl+Z` | Desfazer |
| `Ctrl+C` / `Enter` | Copiar e fechar |
| `Esc` | Cancelar |

Arraste as bordas ou os cantos para redimensionar. Cada traço guarda a própria cor, e todo print começa em vermelho.

![Paleta de cores aberta](docs/cores.png)

## Gravação

**Win+R** e arraste uma caixa. A gravação começa quando você solta o mouse.

**Win+R** de novo para. O vídeo vai para `~/Vídeos/Ahri/` e o arquivo já fica copiado, pronto para colar no Discord, no navegador ou no gerenciador de arquivos.

![Área sendo gravada](docs/gravando.png)

A moldura fica fora da área gravada e não bloqueia cliques.

## Instalação

Precisa de KDE Plasma 6 no Wayland, Rust e [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/).

```sh
sudo pacman -S gpu-screen-recorder
git clone https://github.com/juicerq/ahri
cd ahri
./install.sh
```

O script instala o binário em `~/.local/bin`, registra o ahri no KWin e liga um serviço systemd de usuário. Win+Shift+S e Win+R passam do Spectacle para o ahri. O Spectacle continua no Print e no Meta+Shift+R.

Para remover:

```sh
systemctl --user disable --now ahri
rm ~/.local/bin/ahri ~/.local/share/applications/ahri.desktop \
   ~/.local/share/icons/hicolor/scalable/apps/ahri.svg ~/.config/systemd/user/ahri.service
```

## Por baixo

- O print vem do `ScreenShot2` do KWin, sem diálogo de permissão.
- O overlay é uma janela layer-shell desenhada na CPU com [tiny-skia](https://github.com/linebender/tiny-skia).
- A gravação usa o `gpu-screen-recorder` em H.264. Ele codifica na GPU quando pode e cai para a CPU quando não pode.
- O clipboard é servido pelo próprio daemon, então o conteúdo continua lá depois que o overlay fecha.
