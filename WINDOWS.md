# VTD Windows

Offline české diktování přes Vulkan. Windows alternativa projektu [MQ37/vtd](https://github.com/MQ37/vtd).

## Spuštění

Rozbal celý balíček do zapisovatelné složky. Spusť `vtd.exe` a počkej na načtení modelu. Program běží na pozadí bez oken, ikony a notifikací.

- Podrž **F8**, mluv, pusť. Text se vloží do původního pole.
- Nebo stiskni **F9** pro začátek a znovu **F9** pro dokončení. Klávesu nemusíš držet.
- Mikrofon po puštění dobíhá 250 ms kvůli koncům slov. Přepis používá beam search s pěti kandidáty.
- **Esc** zruší záznam nebo zabrání vložení rozpracovaného přepisu.
- `vtd status` vypíše stav, `vtd stop` program ukončí, `vtd copy` zkopíruje poslední přepis do schránky.
- Přepis zůstává v paměti do dalšího výsledku nebo ukončení. Zvuk se neukládá.
- Když během diktování nebo přepisu klikneš jinam či píšeš, výsledek se automaticky nevloží. Zkopíruj ho příkazem `vtd copy`.

Model musí být v `models/ggml-large-v3-turbo-q5_0.bin`. Pokud balíček model neobsahuje, spusť ze složky balíčku:

```powershell
powershell -ExecutionPolicy Bypass -File .\download-model.ps1 -Destination .\models
```

Po stažení není potřeba připojení. AMD ovladač musí obsahovat Vulkan. Python, Rust, Vulkan SDK ani ROCm nejsou pro běh potřeba.

## Nastavení

`vtd.json` leží vedle programu. Po změně VTD ukonči a znovu spusť.

| Položka | Výchozí hodnota | Význam |
| --- | --- | --- |
| `language` | `cs` | Jazyk přepisu |
| `trigger_key` | `119` | F8; povolené F1–F24: 112–135 |
| `toggle_key` | `120` | F9; začátek/konec jedním stiskem, musí se lišit od `trigger_key` |
| `toggle` | `false` | `true`: jednou stisknout pro začátek, podruhé pro konec |
| `clipboard_paste` | `false` | `true`: vložit celý přepis přes schránku a Ctrl+V; přepis nahradí obsah schránky |
| `microphone` | `null` | Výchozí mikrofon; jinak přesný název z `vtd devices` |
| `gpu` | `null` | Automaticky samostatná GPU, jinak první dostupná; lze zadat index |
| `idle_unload_seconds` | `300` | Po 5 minutách uvolnit model; `0` ho ponechá načtený |
| `max_recording_seconds` | `120` | Nejdelší jeden záznam |
| `silence_rms` | `0.002` | Práh pro vynechání ticha; nižší pro velmi tichý mikrofon |
| `threads` | `4` | Počet CPU vláken pro přepis |
| `model` | cesta k Q5 | Relativně ke konfiguraci, nebo absolutní cesta |

`vtd devices` vypíše GPU a mikrofony. `vtd autostart on` zapne spuštění po přihlášení, `vtd autostart off` ho vypne. Instalace autostart sama nezapíná.

## Omezení

Výchozí vkládání používá Unicode SendInput a nemění schránku. Pokud editor ztrácí nebo opakuje znaky, nastav `clipboard_paste: true`. Tento režim vloží text přes Ctrl+V a ponechá ve schránce poslední přepis; předchozí obsah neobnovuje. VTD spuštěné bez správce nemůže vkládat do aplikací spuštěných jako správce. Program si oprávnění sám nezvyšuje.

Kontrola okna a aktivity výrazně omezuje vložení jinam, ale není transakcí s cílovým editorem. Během vkládání neměň fokus. Filtr ticha není plnohodnotný rozpoznávač řeči; hluk může vyvolat chybný přepis.

## Build

Potřeba: Rust stable s MSVC targetem, Visual Studio C++ Build Tools a Windows SDK, CMake/Ninja, Vulkan SDK a libclang. Nastav `VULKAN_SDK` a `LIBCLANG_PATH`. Lokální nástroje lze také uložit do `.tools` podle `scripts/build-windows.ps1`.

`scripts/setup-windows.ps1` stáhne ověřený Vulkan SDK a libclang do `.tools`. Vyžaduje Python a 7-Zip; nemění systémový ovladač Vulkan.

```powershell
.\scripts\build-windows.ps1
.\scripts\build-windows.ps1 -Test
.\scripts\download-model.ps1
.\scripts\package-windows.ps1 -WithModel
```

Krátká výchozí cesta `C:\vtd-build` obchází limit délky cest nástrojů Windows; lze změnit `-BuildDir`. `GGML_NATIVE=OFF` brání optimalizaci pouze pro CPU buildovacího počítače. Vulkan vybírá zařízení za běhu. Jeden x64 balíček je určen pro RX 7800 XT i Strix Halo; druhý stroj vyžaduje vlastní ověření.

`vtd transcribe nahravka.wav 5` změří opakované přepisy s jednou načteným modelem. Diagnostika jde na stderr, text na stdout. Běžné diktování text neloguje; diagnostika obsahuje název mikrofonu, délku zachyceného zvuku, dobu záznamu a RMS hlasitost.

Původní linuxový program a jeho instalační skripty jsou zachovány. Nová část je v `src/windows`.

Pro jednorázovou diagnostiku lze spustit `vtd run --capture-next test.wav`. Uloží pouze první dokončený záznam (mono, 16 kHz, float WAV) lokálně; existující soubor nepřepíše. Běžné spuštění nic neukládá. Dekodér používá standardní teplotní fallback při neúspěšném dekódování.
