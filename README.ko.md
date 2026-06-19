# Zlack (즐랙)

**Zlack**은 [Slack](https://slack.com/)을 위한 가볍고 최적화된 데스크탑 래퍼 애플리케이션으로, [Tauri](https://tauri.app/)를 기반으로 제작되었습니다. 시스템 트레이 최소화 상태에서도 딥링크(Deep Link) 처리와 윈도우 포커싱이 올바르게 작동하며, 강력한 네이티브 데스크탑 알림 기능을 제공합니다.

![Zlack 아이콘](src-tauri/icons/128x128.png)

## 🚀 주요 기능

*   **네이티브 데스크탑 알림**: Windows 네이티브 토스트 알림과 직접 연동됩니다.
*   **안 읽음 트레이/제목 뱃지**: 트레이 아이콘에 안 읽은 DM/@멘션은 빨간 뱃지, 그 외 안 읽은 메시지는 파란 뱃지로 표시하고, 안 읽은 DM이 있으면 창 제목 앞에 `!`를 붙입니다. 토스트 팝업보다 은은한 표시를 선호하는 분께 유용합니다.
*   **방화벽 친화적 빌드 (선택)**: Windows 빌드 변형에서 고정 버전 WebView2 런타임을 Zlack 디렉토리 안에 전용으로 포함시켜, 방화벽 규칙을 Zlack에만 한정할 수 있습니다.
*   **스마트 컨텍스트 감지**: Slack의 네트워크 로그를 분석하여 `Team ID`와 `Channel ID`를 추출, 알림 클릭 시 정확한 채널로 이동합니다.
*   **백그라운드 안정성**: Rust 백엔드를 통해 알림 클릭 시 시스템 트레이에 숨겨진 윈도우를 안정적으로 복구하고 포커싱합니다.
*   **멀티 워크스페이스 지원**: 표준 웹뷰 로그인을 통해 여러 슬랙 워크스페이스 간 네비게이션을 지원합니다.
*   **가벼운 성능**: 무거운 Chromium 번들(Electron) 대신 Tauri의 최소화된 풋프린트(Windows의 경우 WebView2)를 사용하여 매우 가볍습니다.

## 🛠 기술 스택

*   **프론트엔드**: 바닐라 HTML/JS (Slack 웹 클라이언트) + `preload.js` (브리지 역할)
*   **백엔드**: Rust (Tauri) - 시스템 통합 담당
*   **알림 엔진**: `tauri-winrt-notification` - 고급 Windows 토스트 기능(Input, Activation Callback 등) 사용

## 📦 설치 방법

운영체제에 맞는 설치 파일을 다운로드하세요:

| 플랫폼 | 파일명 |
|----------|------|
| **Windows** | `Zlack_${version}_x64-setup.exe` (설치 프로그램) 또는 `Zlack_${version}_x64_en-US.msi` |
| **macOS** | `Zlack_${version}_x64.dmg` |

1.  설치 파일을 실행합니다.
2.  시작 메뉴에서 **Zlack**을 실행합니다.
3.  Slack 워크스페이스에 로그인합니다.

## 🏗 개발 가이드

### 필수 요구사항

*   [Node.js](https://nodejs.org/)
*   [Rust & Cargo](https://rustup.rs/)
*   [Tauri CLI](https://tauri.app/v1/guides/getting-started/prerequisites)

### 명령어

**의존성 설치:**
```bash
npm install
```

**개발 모드로 실행:**
```bash
npm run tauri dev
```
*참고: `dev` 모드에서는 Windows AUMID 제약으로 인해 알림 클릭 시 윈도우 복구가 불안정할 수 있습니다. 이는 정식 빌드(Release) 버전에서 완벽하게 작동합니다.*

**프로덕션 빌드 (Windows):**
```bash
npm run build:dist:windows
```
이 명령어는 애플리케이션을 컴파일하고 `dists/` 폴더에 설치 파일(`.exe` 및 `.msi`)을 생성합니다.

**프로덕션 빌드 (macOS/Linux):**
```bash
npm run build:dist:unix
```
Mac 또는 Linux 머신에서 실행해야 합니다. `dists/` 폴더에 `.dmg`/`.app` (macOS) 또는 `.deb`/`.AppImage` (Linux) 파일을 생성합니다.

**고정 WebView2 런타임 포함 빌드 (Windows):**
```bash
npm run build:dist:windows:fixed
```
기본 Zlack은 시스템 공유 WebView2 런타임을 사용합니다. 이 런타임은 공유 자원이라 방화벽에서 허용하면 *어떤 앱이든* 이를 통해 인터넷에 나갈 수 있습니다. 이 변형 빌드는 **고정 버전 WebView2 런타임을 Zlack 디렉토리 안에 전용 사본**으로 포함시켜, 방화벽 규칙을 Zlack에만 한정할 수 있게 합니다(이슈 #2). 대신 설치 파일이 약 150~180MB 커집니다.

런타임은 한 번만 받으면 되며 저장소에 커밋되지 않습니다. [WebView2 다운로드 페이지](https://developer.microsoft.com/microsoft-edge/webview2/)에서 아키텍처에 맞는 **Fixed Version** `.cab`을 받은 뒤 실행하세요:
```powershell
# cab 경로를 직접 전달하거나...
npm run build:dist:windows:fixed -- "C:\경로\Microsoft.WebView2.FixedVersionRuntime.<ver>.x64.cab"
# ...환경 변수나 직접 URL로 지정:
$env:WEBVIEW2_FIXED_CAB = "C:\경로\to.cab"; npm run build:dist:windows:fixed
$env:WEBVIEW2_FIXED_URL = "https://.../...cab"; npm run build:dist:windows:fixed
```
`src-tauri/webview2-runtime/`에 한 번 추출되면 이후 빌드에서 재사용됩니다. 결과 설치 파일에는 `_webview2fixed` 접미사가 붙습니다.

## 🧩 작동 원리

### 알림 가로채기 (Notification Interception)
Slack 웹 클라이언트는 `/traces/v1/list_of_spans`로 텔레메트리 데이터를 전송합니다. Zlack의 `preload.js`는 이 네트워크 트래픽을 가로챕니다:
1.  `notification:sent` 스팬(span)을 캡처하여 해당 이벤트와 관련된 `Team ID`와 `Channel ID`를 확실하게 식별합니다.
2.  브라우저의 `Notification` API 요청을 가로챕니다.
3.  알림 내용에 위에서 캡처한 컨텍스트 정보를 병합하여 Rust 백엔드로 전송합니다.

### 강력한 윈도우 복구 (Robust Window Restoration)
앱이 최소화된 상태에서 Windows 알림을 클릭했을 때 창을 띄우는 것은 OS의 포커스 정책 때문에 매우 까다롭습니다. Zlack은 다음과 같이 이를 해결했습니다:
1.  **메인 스레드 아키텍처**: 알림 객체를 메인 스레드에서 생성하여 COM 리스너가 앱이 실행되는 동안 계속 유지되도록 보장합니다.
2.  **단계적 복구**: `set_skip_taskbar(false)`, `unminimize()`, `show()`를 순서대로 명시적으로 호출합니다.
3.  **포커스 해킹**: Windows가 포커스를 차단하더라도 강제로 창을 맨 앞으로 가져오기 위해 일시적으로 "항상 위에 표시(Always On Top)" 속성을 토글하는 기법을 사용합니다.

## 📄 라이선스

MIT
