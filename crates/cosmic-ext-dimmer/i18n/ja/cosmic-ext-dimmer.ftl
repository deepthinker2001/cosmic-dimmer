applet-name = 調光

looking = ディスプレイを検索中…
display-unnamed = ディスプレイ { $number }
no-displays = ディスプレイが見つかりません。

level = { $value }%

software-unavailable = ソフトウェア調光は使用できません: { $reason }
reason-not-found = ディスプレイが見つかりません
reason-read-only = ディスプレイが変更を受け付けません
reason-unsupported = このビルドではサポートされていません
reason-timed-out = 時間内に応答がありません
reason-invalid-level = レベルが範囲外です
reason-unavailable = 現在ディスプレイに接続できません
reason-superseded = 新しいレベルに置き換えられました

not-applied = 適用されませんでした: { $reason }
not-applied-at = 適用されませんでした: { $reason }。ディスプレイは { $value }% です。

lock = 両方を一緒に動かす

auto-dimming = 自動調光
location = 日の出・日の入りを計算する位置
location-time-zone = タイムゾーンから ({ $zone })
location-placeholder = 緯度, 経度
location-invalid = 位置ではありません。緯度, 経度を度単位で入力してください (例: 39.74, -104.99)。
location-unknown = お使いのタイムゾーンには既知の位置がありません。自動調光を使うには緯度, 経度を入力してください。

settings = 設定
floor-hardware = ハードウェアの最低レベル
floor-software = 最低輝度
floor-effective = 合計の最低レベル
key-step = キー 1 回あたりの変化量

keys-unavailable = 輝度キーは使用できません: { $error }
shortcuts-toggle = 輝度ショートカット
shortcut-raise = 輝度を上げるキー
shortcut-lower = 輝度を下げるキー
shortcut-press = キーを押してください…
shortcut-hint = F キー、または Super、Ctrl、Alt との組み合わせを押してください。Esc でキャンセルします。COSMIC 自体が使うキーは登録されません。
shortcuts-taken = 変更されませんでした: { $binding } はすでに使われています。
shortcuts-invalid = 変更されませんでした: そのキーの組み合わせはショートカットに使えません。
shortcut-description-raise = 調光: 輝度を上げる
shortcut-description-lower = 調光: 輝度を下げる
shortcuts-unreadable = 変更されませんでした: カスタムショートカットを安全に読み取れなかったため、そのままにしました。
shortcuts-failed = 変更されませんでした: { $error }

usage =
    使い方: cosmic-ext-dimmer [--reset | --help | --version]
    オプションなしでは COSMIC パネルアプレットとして動作します。
    --reset  すべてのディスプレイを 100% (ハードウェアとソフトウェア調光) に戻して
             保存します。画面が暗すぎてアプレットを使えないときに使います。
reset-applet-done = 実行中のアプレットがすべてのディスプレイを 100% にし、オーバーレイを削除しています。
reset-applet-not-running = アプレットは実行されていないため、画面にソフトウェア調光はかかっていません。
reset-applet-no-answer = アプレットが応答しませんでした ({ $error })。画面がまだ暗い場合は、次のコマンドでアプレットを終了してください: pkill -f 'cosmic-ext-dimmer$'
reset-saved = { $count } 台のディスプレイの保存済みレベルを 100% にしました。
reset-saved-failed = 保存済みレベルはリセットされませんでした: { $error }
reset-saved-no-home = 保存済みレベルはリセットされませんでした: HOME が設定されていないため、設定フォルダーが見つかりません。
reset-hardware = { $count } 台のディスプレイでハードウェア輝度を 100% にしました。
reset-hardware-partial = { $count } 台のディスプレイでハードウェア輝度を 100% にしました。{ $failed } 台は反映されませんでした。
reset-hardware-skipped = ハードウェア輝度は変更されませんでした: { $reason }。
reset-failed = リセットに失敗しました: { $error }
