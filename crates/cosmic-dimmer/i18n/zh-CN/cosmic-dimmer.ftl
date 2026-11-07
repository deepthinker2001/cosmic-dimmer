applet-name = 调光

looking = 正在查找显示器…
display-unnamed = 显示器 { $number }
no-displays = 未找到显示器。

level = { $value }%

software-unavailable = 软件调光不可用：{ $reason }
reason-not-found = 未找到显示器
reason-read-only = 显示器不接受更改
reason-unsupported = 此版本不支持
reason-timed-out = 未及时响应
reason-invalid-level = 级别超出范围
reason-unavailable = 暂时无法连接显示器
reason-superseded = 已被更新的级别取代

not-applied = 未应用：{ $reason }
not-applied-at = 未应用：{ $reason }。显示器当前为 { $value }%。

lock = 一起移动

auto-dimming = 自动调光
location = 用于日出和日落的位置
location-time-zone = 来自时区（{ $zone }）
location-placeholder = 纬度, 经度
location-invalid = 不是有效位置。请以度为单位输入纬度, 经度，例如 39.74, -104.99。
location-unknown = 你的时区没有已知位置。请输入纬度, 经度以使用自动调光。

settings = 设置
floor-hardware = 最低硬件级别
floor-software = 最低亮度
floor-effective = 最低组合级别
key-step = 每次按键的变化量

keys-unavailable = 亮度按键不可用：{ $error }
shortcuts-toggle = 亮度快捷键
shortcut-raise = 增加亮度按键
shortcut-lower = 降低亮度按键
shortcut-press = 请按键…
shortcut-hint = 按下一个 F 键，或与 Super、Ctrl、Alt 的组合。按 Esc 取消。COSMIC 自身使用的按键不会被注册。
shortcuts-taken = 未更改：{ $binding } 已被使用。
shortcuts-invalid = 未更改：该组合键不能用作快捷键。
shortcut-description-raise = 调光：增加亮度
shortcut-description-lower = 调光：降低亮度
shortcuts-unreadable = 未更改：无法安全读取你的自定义快捷键，因此保持原样。
shortcuts-failed = 未更改：{ $error }

usage =
    用法: cosmic-dimmer [--reset | --help | --version]
    不带选项时作为 COSMIC 面板小程序运行。
    --reset  将每个显示器恢复到 100%（硬件和软件调光）并保存，
             用于屏幕太暗而无法使用小程序的情况。
reset-applet-done = 正在运行的小程序正在将每个显示器设为 100% 并移除其叠加层。
reset-applet-not-running = 小程序未运行，因此屏幕上没有软件调光。
reset-applet-no-answer = 小程序没有响应（{ $error }）。如果屏幕仍然变暗，请用以下命令结束小程序：pkill -f 'cosmic-dimmer$'
reset-saved = 已将 { $count } 个显示器的已保存级别设为 100%。
reset-saved-failed = 已保存的级别未重置：{ $error }
reset-saved-no-home = 已保存的级别未重置：未设置 HOME，因此找不到设置文件夹。
reset-hardware = 已将 { $count } 个显示器的硬件亮度设为 100%。
reset-hardware-partial = 已将 { $count } 个显示器的硬件亮度设为 100%；{ $failed } 个未生效。
reset-hardware-skipped = 硬件亮度未更改：{ $reason }。
reset-failed = 重置失败：{ $error }
