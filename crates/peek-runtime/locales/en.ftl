app-name = Crant Peek
header-pin = Pin
header-unpin = Unpin
header-settings = Settings
header-close = Close
action-close = Close
action-pin = Pin as a regular window
action-unpin = Back to the quick peek
header-back = Back
header-paused = Shortcuts paused
query-placeholder = Type or paste something…
query-run = Look up
query-stop = Stop
query-copy = Copy result
query-copy-source = Copy source
query-clear = Clear
query-restore-smart = Auto mode
query-target = Target language
query-target-auto = Automatic
query-target-custom = Custom language
query-followup-placeholder = Ask a follow-up…
query-send = Send
query-empty-title = Here when you need it. Gone when you don't.
query-empty-hint = Select text and double-tap Ctrl, or type above. { $submit } to look up.
query-section-answer = AI answer
task-auto = Auto
task-auto-tooltip = Decide the task from the content
task-translate = Translate
task-define = Dictionary
task-explain-code = Explain code
task-explain-error = Diagnose error
task-explain = Explain
dict-badge = Offline dictionary
dict-lemma = Lemma: { $lemma }
form-past = Past tense
form-past-participle = Past participle
form-present-participle = Present participle
form-third-person = Third person
form-comparative = Comparative
form-superlative = Superlative
form-plural = Plural
snip-copy-text = Copy text
snip-to-input = Send to input
snip-copied = Source text copied
snip-hint = Drag to select · Esc to cancel
snip-image-title = Screenshot selection
snip-explain = Explain image
snip-decide = Decide with Clef
snip-discard = Discard image
snip-explain-default = Explain the content of this screenshot.
snip-decide-default = Choose the appropriate reading assistance for this screenshot.
snip-decide-target = Clef sends only to the decision service: { $endpoint }
snip-explain-target = Image explanations go to { $endpoint }; follow-ups send text only
snip-cancel-ocr = Cancel OCR
route-local = Local routing · manual mode
route-deciding = Choosing a task…
route-decided = { $model } · confidence { $confidence }
route-uncertain = Uncertain decision · using local routing
route-unavailable = Decision unavailable · using local routing
status-key-missing = This channel has no API key; add one in settings
status-channel-missing = No usable channel yet; add one in settings
status-channel-incomplete = Channel is incomplete: name and endpoint are required
status-generating = Generating…
status-failed = Request failed
status-done = Done
status-done-trimmed = Done · older follow-ups released; original query and recent turns retained
status-stopped = Stopped
status-copied = Copied
status-copied-source = Source copied
status-cleared = Current session cleared
status-saved = Saved
status-saved-restart = Saved. Restart to apply shortcut changes.
status-input-too-long = Input exceeds 64 KiB. Select less text or split the query.
status-output-too-long = Answer exceeded 512 KiB and was stopped. Narrow the question.
status-model-missing = Configure an answer model in Settings first.
status-worker-crashed = Network task stopped unexpectedly. Please retry.
status-ocr-running = Recognizing text locally…
status-ocr-cancelled = OCR cancelled
status-ocr-empty = No text detected. Select another region.
status-ocr-failed = OCR failed: { $error }
status-ocr-local-only = Local OCR complete. Copy or query manually; nothing has been sent to a model.
status-capture-failed = Capture failed. Check screen recording permission: { $error }
status-paused = Shortcuts paused. Resume from the tray menu.
status-resumed = Shortcuts resumed
status-config-error = Cannot load configuration: { $error }
status-desktop-error = Desktop integration failed: { $error }
error-key-missing = No API key configured. Open Settings.
error-keychain-open = Cannot open the system credential store.
error-keychain-save = Cannot save credentials to the system store.
error-network = Network request failed: { $detail }
error-http = Service returned HTTP { $code }
error-invalid-response = Invalid service response: { $detail }
error-cancelled = Request cancelled
error-truncated = Answer reached the token limit and may be incomplete. Increase the limit and retry.
error-content-filter = Answer stopped by the service's content filter.
error-service = The service reported an error.
error-hotkey-blank = Invalid blank-window shortcut: { $detail }
error-hotkey-screenshot = Invalid screenshot shortcut: { $detail }
error-hotkey-duplicate = The two entries cannot use the same shortcut.
error-config-version = Unsupported configuration version.
error-config-double-ctrl = Double Ctrl interval must be 150–800 ms.
error-config-shortcut-same = Entry shortcuts must differ.
error-config-language = Target language cannot be empty.
error-config-appearance = Invalid appearance settings.
error-config-style = Invalid translation style.
error-config-tokens = Output token limit must be 128–16384.
error-config-endpoint = Invalid endpoint: use HTTPS or loopback HTTP, without embedded credentials.
error-config-decision-model = Decision model cannot be empty.
error-config-decision-timeout = Decision timeout must be 100–10000 ms.
error-config-decision-threshold = Invalid decision confidence threshold.
settings-decision = Decision model
settings-decision-enabled = Enabled (classify content; fall back to local rules)
settings-tab-shortcuts = Shortcuts
settings-shortcuts-hint = Format such as Super+Shift+A (Super is Command on macOS). A restart applies changes.
settings-tab-appearance = Appearance
settings-tab-translation = Translation
settings-tab-channels = Channels
settings-tab-permissions = Permissions
settings-title = Settings
settings-save = Save
settings-cancel = Cancel
settings-section-general = General
settings-section-appearance = Appearance
settings-section-shortcuts = Shortcuts
settings-section-answer = Answer model
settings-section-decision = Decision model
settings-section-translation = Translation
settings-section-privacy = Privacy & permissions
settings-language = Interface language
settings-language-system = Follow system
settings-theme = Theme
settings-theme-system = Follow system
settings-theme-light = Light
settings-theme-dark = Dark
settings-zoom = Interface scale
channel-kind-chat = OpenAI Chat Completions
channel-kind-responses = OpenAI Responses
channel-kind-anthropic = Anthropic Messages
channel-kind-deeplx = DeepLX
channel-kind-decision = Decision model
channels-add-title = Add a channel
channels-edit-title = Edit channel
channels-edit = Edit
channels-save = Save
channels-cancel = Cancel
channels-key-keep = Leave empty to keep the stored key
channels-title = Channels
channels-empty = No channels yet; add one first
channels-test = Test connection
channels-testing = Testing…
channels-test-ok = Connected
channels-test-failed = Failed: { $detail }
channels-add = Add
channels-remove = Remove
channels-kind = Type
channels-name = Name
channels-endpoint = Endpoint
channels-key = API key
channels-model = Model ID
channels-usage = Used for
channels-basic = Basic translation
channels-ai = LLM
channels-decision = Decision model
channels-decision-result = Decision: { $task }
channels-decision-hint = Full URL, such as a Cloudflare …/ai/run/@cf/cloudflare/clef-flash route, or TypeSafe /v1/systemone. Model is clef, clef-flash, or jev-latest.
channels-none = Not selected
channels-basic-pick = Basic translation channel
error-config-channel-model = A channel is missing its model ID
error-config-channel-missing = A selected channel is missing or cannot serve that place
error-translation-size = Translation response is too large
settings-font-set = Font set
settings-shortcut-blank = Blank window
settings-shortcut-selection = Ask about the selection
settings-shortcut-screenshot = Screenshot
settings-double-ctrl = Double Ctrl interval (ms)
settings-shortcut-hint = Example: Super+Shift+A (Command on macOS), Alt+Shift+A (Windows). Restart to apply changes.
settings-protocol = Protocol
settings-base-url = Base URL
settings-base-url-hint = Usually includes /v1
settings-model = Model
settings-max-tokens = Output token limit
settings-vision = Supports images (selection uploaded only on explicit image explanation)
settings-api-key = API key
settings-api-key-hint = Leave blank to keep saved credentials.
settings-test = Test connection
settings-test-cost = Uses a small amount of API quota.
settings-test-cancel = Cancel test
settings-test-running = Testing…
settings-test-ok = Connected · streaming response verified
settings-test-failed = Connection failed: { $error }
settings-test-timeout = Connection test timed out.
settings-test-crashed = Connection test task failed.
settings-test-cancelled = Connection test cancelled.
settings-decision-enable = Enable a dedicated decision service
settings-decision-endpoint = Endpoint
settings-decision-endpoint-hint = Full URL: TypeSafe or a Cloudflare model route
settings-decision-model = Decision model
settings-decision-key = Decision API key / token
settings-decision-threshold = Confidence threshold
settings-decision-timeout = Timeout (ms)
settings-decision-note = Only current query text is sent. Falls back to local routing if unavailable.
settings-default-target = Default target language
settings-chinese-target = Translate Chinese into
settings-style = Translation style
settings-style-natural = Natural
settings-style-literal = Literal
settings-style-technical = Technical documentation
settings-smart-mode = Automatically choose the task
settings-ocr-auto = Query automatically after OCR
settings-ocr-auto-hint = Turn off for local-only OCR. External services run only when you click query or image actions.
settings-hide-on-blur = Hide when focus is lost
settings-snip-close-on-copy = Close the screenshot window after copying
settings-permissions = System permissions
settings-permission-granted = Granted
settings-permission-denied = Not granted
settings-permission-accessibility = Accessibility
settings-permission-accessibility-purpose = Read the text selection
settings-permission-input = Input monitoring
settings-permission-input-purpose = Hotkey listening
settings-permission-screen = Screen recording
settings-permission-screen-purpose = Screenshots
settings-permission-hint = Double Ctrl reads selections only. If the app does not expose selections, paste into a blank window or capture a region.
settings-permission-windows = Elevated apps and secure inputs are inaccessible; OS OCR needs installed language packs.
settings-permission-restart = You may need to restart Peek after changing macOS permissions.
settings-open-privacy = Open system privacy settings
settings-permission-refresh = Check again
settings-open-privacy-failed = Cannot open system settings.
welcome-title = Welcome to Crant Peek
welcome-selection = Double Ctrl queries selected text only. No selection, no popup.
welcome-shortcuts = { $blank }: blank input · { $screenshot }: screenshot
welcome-dismiss = Esc or focus loss hides the window. Pin it for comparison. Reopen or quit from the tray.
welcome-privacy = Dictionary and OCR run locally. AI query text goes to your configured services; clipboard and full screen are never uploaded automatically.
welcome-start = Get started
tray-tooltip = Crant Peek
tray-open = Open blank Peek
tray-screenshot = Screenshot
tray-settings = Settings
tray-pause = Pause / resume shortcuts
tray-quit = Quit

language-zh = 简体中文
language-en = English
language-ja = Japanese

error-image-size = Image exceeds the upload size limit.

error-image-base64 = Invalid image encoding.

error-image-header = Invalid PNG image header.

error-image-pixels = Image exceeds the pixel limit.

error-image-disabled = Image input is not enabled for this provider.

error-message-missing = Request is missing messages.

error-image-message = Image requests require a user message.

error-sse-buffer = Stream buffer exceeds the limit.

error-sse-encoding = Invalid stream encoding.

error-sse-event = Stream event exceeds the limit.

error-sse-json = Malformed stream data.

error-response-incomplete = Response failed or is incomplete.

error-sse-type = Service did not return an event stream.

error-stream-ended = Stream ended unexpectedly; content may be incomplete.

error-output-size = Generated output exceeds the size limit.

error-decision-size = Decision response exceeds the size limit.

error-decision-json = Malformed decision response.

error-decision-schema = Invalid decision response schema.

error-decision-image-model = Image decisions require Clef or Clef Flash.

error-decision-probabilities = Invalid decision probabilities.

error-decision-missing-task = Chosen task is missing from probabilities.

error-decision-unknown-task = Decision contains an unknown task.

error-decision-inconsistent = Decision choice is inconsistent with probabilities.
error-decision-timeout = Decision timed out. The local choice is used instead.

error-config-directory = Cannot locate a valid configuration directory.
error-ocr-language-pack = Install an OCR language pack in Windows Settings.
error-no-display = No display is available.

preview-answer = A reading assistant should respect your attention.
    
    Understand the text in front of you, then step aside.

protocol-chat = OpenAI · Chat Completions
protocol-responses = OpenAI · Responses
protocol-anthropic = Anthropic · Messages

preview-source = The best tools respect your attention.
error-instance = Cannot acquire application instance lock: { $detail }
