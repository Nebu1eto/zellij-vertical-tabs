# zellij-vertical-tabs

[English](README.md) | 한국어

가로 상태 표시줄과 세로 사이드바를 제공하는 Zellij WebAssembly 플러그인입니다. 탭, 저장소, 브랜치, 연결된 워크트리 상태와 시계, choco-pi, Claude Code, Codex 및 터미널에서 감지한 코딩 에이전트의 상태를 표시합니다.

![탭과 코딩 에이전트를 표시하는 가로 상태 표시줄과 세로 사이드바](assets/demo.png)

## 설치

두 릴리스 파일을 다운로드합니다. 두 파일은 같은 플러그인 바이너리지만, Zellij가 URL을
기준으로 플러그인 인스턴스를 구분할 수 있도록 서로 다른 이름을 사용합니다.

```sh
mkdir -p ~/.config/zellij/plugins
curl -fL https://github.com/Nebu1eto/zellij-vertical-tabs/releases/latest/download/vertical-tabs.wasm \
  -o ~/.config/zellij/plugins/vertical-tabs.wasm
curl -fL https://github.com/Nebu1eto/zellij-vertical-tabs/releases/latest/download/vertical-sidebar.wasm \
  -o ~/.config/zellij/plugins/vertical-sidebar.wasm
```

처음 실행할 때 각 플러그인 창에 포커스를 두고 `y`를 눌러 요청된 권한을 허용합니다.

## 설정

다음 레이아웃은 서로 다른 경로에서 가로 보기와 세로 보기를 불러옵니다. 정확한 열 너비를 유지하면서 크기를 조절할 수 있도록 사이드바 pane의 size는 지정하지 않습니다.

```kdl
layout {
    pane size=2 borderless=true {
        plugin location="file:~/.config/zellij/plugins/vertical-tabs.wasm" {
            view "horizontal"
            show_tabs "true"
            timezone_offset_hours "9"
        }
    }
    pane split_direction="vertical" {
        pane
        pane borderless=true {
            plugin location="file:~/.config/zellij/plugins/vertical-sidebar.wasm" {
                view "vertical"
                initial_width "28"
                home "/Users/example"
                vertical_separator_enabled "true"
                vertical_separator_char "│"
            }
        }
    }
}
```

주요 설정 키는 다음과 같습니다.

| 키 | 값 또는 용도 |
| --- | --- |
| `view` | `"horizontal"` 또는 `"vertical"` |
| `show_tabs` | 가로 탭 표시 여부. 기본값은 `"true"` |
| `timezone_offset_hours` | UTC 기준 시계 오프셋(시간) |
| `home` | 작업 디렉터리를 표시할 때 사용할 홈 경로 |
| `initial_width` | 세로 사이드바의 초기 열 너비. 크기 조절을 유지하려면 레이아웃 pane의 size를 지정하지 않음 |
| `vertical_separator_enabled` | 사이드바 구분선 표시 여부. 기본값은 `"true"` |
| `vertical_separator_char` | 사이드바 구분선 문자. 기본값은 `"│"` |
| `border_enabled` | 사이드바 구분선과 별개인 가로 보기 테두리 사용 여부 |
| `border_char` | 가로 보기 테두리 문자 |
| `right_panel` | 가로 막대 오른쪽 끝: `"clock"`(기본값), `"music"`, `"both"`. music 옵션은 macOS 전용이며 다른 OS에서는 시계로 대체 |
| `music_format` | 재생 중 텍스트. 자리표시자 `{music}`(또는 `{title}`), `{artist}`, `{album}`, `{albumart}`. 기본값은 `"{artist} - {music}"` |
| `music_max_width` | 재생 중 텍스트의 최대 열 너비. 초과하면 `…`로 자름. 기본값은 `"40"` |
| `center_anchor` | 가로 막대 가운데의 탭·컨텍스트·에이전트 상태를 맞추는 기준. `"bar"`(기본값)는 막대 중앙, `"content"`는 활성 탭에서 세로 사이드바를 제외한 pane 영역의 중앙. 어느 쪽이든 좌우 구간과 겹치면 옆으로 밀림 |

### 재생 중 표시

`right_panel "music"` 또는 `"both"`를 지정하면 Apple Music이 재생 중이거나 일시정지한 트랙을 막대에 표시합니다. Music의 AirPlay 선택 창에서 **컴퓨터** 항목의 스피커로 재생하는 경우도 포함됩니다. **홈** 항목에서 HomePod이나 Apple TV를 선택하면 해당 기기가 직접 Apple Music을 재생하고 Music은 리모컨 역할만 하므로, macOS가 Mac의 플레이어를 보고하지 않아 표시가 비어 있습니다. Music이 정지되어 있거나 실행 중이 아닐 때도 표시하지 않으며, 플러그인이 Music을 실행하지는 않습니다. Zellij가 호스트에서 `osascript`로 Music에 접근하므로 macOS가 Zellij 서버에 대한 자동화 권한을 한 번 묻습니다. 거부하면 표시가 비어 있습니다. 폴링 주기는 트랙에 맞춰 조정됩니다. 재생 중에는 남은 시간의 절반(3–20초), 그 외에는 10초입니다.

`{albumart}`는 현재 `♪`로 표시됩니다. Zellij 0.45는 플러그인 pane의 kitty 그래픽 프로토콜을 버리고 Ghostty는 sixel을 지원하지 않아 아직 앨범 아트를 그릴 방법이 없습니다.

색상에는 `#RRGGBB` 또는 `RRGGBB` 형식을 사용합니다. 값이 올바르지 않으면 내장 Nord 기본값을 사용합니다. 다음 키를 설정할 수 있습니다.

```text
color_background
color_session_fg  color_session_bg
color_mode_normal_fg  color_mode_normal_bg
color_mode_locked_fg  color_mode_locked_bg
color_mode_resize_fg  color_mode_resize_bg
color_mode_pane_fg  color_mode_pane_bg
color_mode_tab_fg  color_mode_tab_bg
color_mode_search_fg  color_mode_search_bg
color_mode_rename_tab_fg  color_mode_rename_tab_bg
color_mode_rename_pane_fg  color_mode_rename_pane_bg
color_mode_move_fg  color_mode_move_bg
color_mode_default_fg  color_mode_default_bg
color_tab_normal_fg  color_tab_normal_bg
color_tab_active_fg  color_tab_active_bg
color_cwd_normal_fg  color_cwd_normal_bg
color_cwd_active_fg  color_cwd_active_bg
color_context_fg  color_context_bg
color_clock_fg  color_clock_bg
color_music_fg  color_music_bg
color_border_fg  color_border_bg
color_agent_fg  color_agent_bg
color_agent_urgent_fg  color_agent_urgent_bg
```

## 스페이스

스페이스는 이름 접두사를 공유하는 탭 묶음입니다. 예를 들어 `work/api`와 `work/web`은 모두 `work` 스페이스에 속합니다. 사이드바는 스페이스 목록을 보여 주고, 가로 막대는 활성 스페이스의 탭만 보여 주며, pane은 평소의 Zellij 분할 그대로입니다. 탭 이름을 바꾸면 그 탭이 다른 스페이스로 옮겨집니다.

이 기능은 기본적으로 꺼져 있습니다. 두 보기 모두에서 켜 주세요.

```kdl
plugin location="file:~/.config/zellij/plugins/vertical-sidebar.wasm" {
    view "vertical"
    spaces "true"
}
```

플러그인은 자기 pane이 포커스를 가진 동안에만 키를 받기 때문에, 스페이스 조작은 키바인드로 전달합니다. `config.kdl`에 아래를 추가하고, 같은 키에 걸려 있던 `GoToTab` 바인딩은 제거하세요. 그 동작은 전역 탭 번호를 가리키므로 스페이스를 가로질러 이동합니다.

```kdl
keybinds {
    normal {
        bind "Super n" { MessagePlugin { name "vtabs:space-new"; }; }
        bind "Super t" { MessagePlugin { name "vtabs:tab-new"; }; }
        bind "Super 1" { MessagePlugin { name "vtabs:space-switch"; payload "1"; }; }
        bind "Super 2" { MessagePlugin { name "vtabs:space-switch"; payload "2"; }; }
        bind "Super Alt Right" { MessagePlugin { name "vtabs:tab-next"; }; }
        bind "Super Alt Left" { MessagePlugin { name "vtabs:tab-prev"; }; }
    }
}
```

`MessagePlugin`에는 플러그인 URL을 적지 않습니다. URL을 적으면 Zellij가 위치와 설정을 함께 비교해 실행 중인 인스턴스를 찾기 때문에, 레이아웃의 모든 설정 키를 그대로 반복하지 않은 바인딩은 사이드바에 닿지 못하고 플러그인 pane을 새로 엽니다.

| 명령 | 동작 |
| --- | --- |
| `vtabs:space-new` | 첫 탭과 함께 새 스페이스 생성 |
| `vtabs:tab-new` | 활성 스페이스에 새 탭 생성 |
| `vtabs:space-switch` | `payload` 번째 스페이스로, 마지막에 머문 탭으로 이동 |
| `vtabs:tab-next`, `vtabs:tab-prev` | 활성 스페이스 안에서 탭 순환 |

`spaces`가 꺼져 있으면 같은 바인딩이 본래 Zellij 동작대로 새 탭, 번호로 탭 이동, 다음/이전 탭으로 작동합니다.

| 키 | 값 또는 용도 |
| --- | --- |
| `spaces` | 탭을 스페이스로 묶기, 기본값 `"false"` |
| `space_separator` | 스페이스와 탭 이름 구분자, 기본값 `"/"` |
| `default_space_name` | 구분자가 없는 탭이 속할 스페이스, 기본값 `"main"` |

### 탭 바

스페이스를 켤 때는 탭 줄을 스페이스 내용 위의 한 줄짜리 pane으로 분리하세요. 상태 표시줄 가운데가 에이전트와 음악용으로 남습니다.

```kdl
pane split_direction="horizontal" {
    pane size=1 borderless=true {
        plugin location="file:~/.config/zellij/plugins/vertical-tabs.wasm" {
            view "tabs"
            spaces "true"
        }
    }
    children
}
```

이 줄은 활성 스페이스의 탭을 왼쪽부터 나열하고 마지막 탭 바로 뒤에 `+` 버튼을 둡니다. 탭을 누르면 이동하고 `+`를 누르면 스페이스에 탭이 추가됩니다. 가로 상태 표시줄에는 `show_tabs "false"`를 두어 탭이 두 번 그려지지 않게 하세요.

## 코딩 에이전트 상태

훅을 설정하지 않아도 플러그인이 지원하는 터미널 에이전트 프로세스를 감지합니다. 훅을 사용하면 다음 이름의 파이프로 JSON을 보내 수명 주기와 작업 세부 정보를 추가할 수 있습니다.

```sh
zellij pipe --name coding-agent-status -- "$payload"
```

JSON에는 창, 이벤트, 도구, 작업 요약, 소스 에이전트, 타임스탬프를 넣을 수 있습니다. choco-pi, Claude Code, Codex의 훅 이벤트는 해당 창의 상태를 갱신합니다.

## 빌드

WASI 타깃을 설치하고 릴리스 파일을 빌드합니다.

```sh
rustup target add wasm32-wasip1
cargo build --release
```

빌드 결과는 다음 경로에 생성됩니다.

```text
target/wasm32-wasip1/release/zellij-vertical-tabs.wasm
```

## 릴리스

CI는 포맷, Clippy, 테스트, 릴리스 WASM 빌드를 검사하고 `vertical-tabs.wasm`과
`vertical-sidebar.wasm`을 포함한 아티팩트를 업로드합니다. 릴리스를 게시하려면 다음
순서로 진행합니다.

1. `Cargo.toml`의 `version`을 `MAJOR.MINOR.PATCH`로 설정합니다.
2. 변경 사항을 커밋하고 푸시합니다.
3. 같은 버전의 태그를 만들고 푸시합니다.

   ```sh
   git tag vMAJOR.MINOR.PATCH
   git push origin vMAJOR.MINOR.PATCH
   ```

릴리스 워크플로는 `Cargo.toml` 버전과 일치하는 엄격한 `vMAJOR.MINOR.PATCH` 형식의
태그만 허용합니다. 조건을 충족하면 공개 GitHub Release를 만들고 기존 빌드 파일과
`vertical-tabs.wasm`, `vertical-sidebar.wasm`을 업로드합니다.

## 라이선스

[MIT](LICENSE)
