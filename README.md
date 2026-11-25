<div align="center">

<img src="assets/icon.svg" width="112" alt="ícone do Ahri">

# Ahri

**Print e gravação de tela para KDE Plasma no Wayland.**<br>
Um daemon em Rust que abre na hora e copia direto para o clipboard.

![Rust](https://img.shields.io/badge/Rust-edition_2024-1d1f25?style=flat-square&logo=rust&logoColor=white)
![KDE Plasma 6](https://img.shields.io/badge/KDE_Plasma-6-1d1f25?style=flat-square&logo=kde&logoColor=white)
![Wayland](https://img.shields.io/badge/Wayland-only-1d1f25?style=flat-square&logo=wayland&logoColor=white)

<br>

<img src="docs/hero.png" alt="Overlay de print do Ahri com traço, seta e barra de ferramentas">

</div>

<br>

## Destaques

- **Abre em cerca de 50 ms.** O daemon já está rodando, e a tela vem direto do KWin, sem diálogo de permissão.
- **Copia na hora.** O PNG vai para o clipboard em cerca de 30 ms. O vídeo vai como arquivo, pronto para colar.
- **Só 4 ferramentas.** Desenho livre, seta, quadrado e círculo, em 6 cores. Nada de menus.
- **Leve.** Sem Qt nem GTK: o overlay é uma janela layer-shell desenhada na CPU.

## Print

Aperte <kbd>Win</kbd> <kbd>Shift</kbd> <kbd>S</kbd>. A tela congela. Arraste uma área, marque o que importa e aperte <kbd>Ctrl</kbd> <kbd>C</kbd>.

<table>
  <tr>
    <td><img src="docs/print.png" alt="Print com traço e seta"></td>
    <td><img src="docs/cores.png" alt="Paleta de cores aberta"></td>
  </tr>
  <tr>
    <td align="center"><sub>Traço livre e seta dentro da área</sub></td>
    <td align="center"><sub>Paleta com 6 cores</sub></td>
  </tr>
</table>

| Tecla | Ação |
| --- | --- |
| <kbd>1</kbd> ou <kbd>A</kbd> | Desenhar |
| <kbd>2</kbd> ou <kbd>D</kbd> | Seta |
| <kbd>3</kbd> ou <kbd>Q</kbd> | Quadrado |
| <kbd>4</kbd> ou <kbd>E</kbd> | Círculo |
| <kbd>5</kbd> | Abrir as cores, depois <kbd>1</kbd>–<kbd>6</kbd> para escolher |
| <kbd>C</kbd> | Próxima cor |
| <kbd>Ctrl</kbd> <kbd>Z</kbd> | Desfazer |
| <kbd>Ctrl</kbd> <kbd>C</kbd> ou <kbd>Enter</kbd> | Copiar e fechar |
| <kbd>Esc</kbd> | Cancelar |

Arraste as bordas ou os cantos para redimensionar a área. Com quadrado ou círculo, arraste dentro da área para definir o tamanho. Segure <kbd>Shift</kbd> para manter os lados iguais. Cada traço guarda a própria cor, e todo print começa em vermelho.

## Gravação

Aperte <kbd>Win</kbd> <kbd>R</kbd> e arraste uma caixa. A gravação começa quando você solta o mouse.

Aperte <kbd>Win</kbd> <kbd>R</kbd> de novo para parar. Um painel abre no meio da tela com o vídeo tocando. Nele você escolhe as faixas de áudio, a qualidade e onde o vídeo começa e termina.

O áudio do sistema e o do microfone são gravados em faixas separadas. Cada faixa mostra a própria onda, então dá para ver se o microfone pegou algo antes de ligar.

O vídeo final é preparado em segundo plano enquanto você assiste. Aperte <kbd>Ctrl</kbd> <kbd>C</kbd>: ele vai para `~/Vídeos/Ahri/`, e o arquivo já fica no clipboard para colar no Discord, no navegador ou no gerenciador de arquivos.

| Tecla | Ação |
| --- | --- |
| <kbd>Espaço</kbd> | Tocar ou pausar |
| <kbd>1</kbd> e <kbd>2</kbd> | Ligar ou desligar o áudio do sistema e o microfone |
| <kbd>↑</kbd> e <kbd>↓</kbd> | Trocar a qualidade |
| <kbd>←</kbd> e <kbd>→</kbd> | Voltar ou avançar 1 segundo |
| <kbd>I</kbd> e <kbd>O</kbd> | Cortar o início ou o fim no ponto atual |
| <kbd>Ctrl</kbd> <kbd>C</kbd> ou <kbd>Enter</kbd> | Salvar e copiar |
| <kbd>Esc</kbd> | Descartar a gravação |

Arraste as alças amarelas da barra para cortar com o mouse, ou clique na barra para pular para um ponto.

<p align="center"><img src="docs/gravando.png" width="600" alt="Área sendo gravada, com a moldura vermelha por fora"></p>

A moldura fica fora da área gravada e não bloqueia cliques.

## Instalação

Precisa de KDE Plasma 6 no Wayland, Rust, [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/) e FFmpeg.

```sh
sudo pacman -S gpu-screen-recorder ffmpeg
git clone https://github.com/juicerq/ahri
cd ahri
./install.sh
```

O script instala o binário em `~/.local/bin`, registra o Ahri no KWin e liga um serviço systemd de usuário. <kbd>Win</kbd> <kbd>Shift</kbd> <kbd>S</kbd> e <kbd>Win</kbd> <kbd>R</kbd> passam do Spectacle para o Ahri. O Spectacle continua no <kbd>Print</kbd> e no <kbd>Win</kbd> <kbd>Shift</kbd> <kbd>R</kbd>.

<details>
<summary>Remover</summary>

```sh
systemctl --user disable --now ahri
rm ~/.local/bin/ahri ~/.local/share/applications/ahri.desktop \
   ~/.local/share/icons/hicolor/scalable/apps/ahri.svg ~/.config/systemd/user/ahri.service
```

</details>

## Por baixo

| Parte | Como |
| --- | --- |
| Captura | `org.kde.KWin.ScreenShot2`, liberado pelo `.desktop` |
| Overlay | layer-shell com [smithay-client-toolkit](https://github.com/Smithay/client-toolkit) e [tiny-skia](https://github.com/linebender/tiny-skia) |
| Atalhos | kglobalaccel por D-Bus |
| Clipboard | `ext-data-control`, servido pelo próprio daemon |
| Gravação | `gpu-screen-recorder` em H.264, na GPU quando dá e na CPU quando não dá |
| Revisão | FFmpeg toca a prévia, desenha a onda e prepara o vídeo final |

<br>

<div align="center">
<sub>O nome é uma homenagem à Ahri, minha gatinha.</sub>
</div>
