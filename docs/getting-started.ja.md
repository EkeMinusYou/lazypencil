# lazypencilをはじめる

lazypencilは、AIと会話しながらUIを作り、ローカルの`.pen`ファイルに保存する
オープンソースのデザインエディターです。実装前の画面案を比較したり、既存の
画面の余白や配色を調整したりできます。設計ファイルはコードと一緒にGitで管理できます。

[English README](../README.md) · [サンプルギャラリー](../examples/README.md)

## 使う前に確認すること

- 配布版の対応環境は、**macOS Tahoe 26以降のApple Silicon Mac**です。
- ターミナル内の画像表示の確認基準は、ローカルで直接起動したKitty 0.49.2と
  WezTerm `20240203-110809-5046fc22`です。他の端末、tmux、SSHなどの対応は別途確認が必要です。
- PNG出力だけなら、ターミナル内の画像表示やAIログインは必要ありません。
- AIを使う場合は、選んだAIクライアント・サービスの利用条件や料金が適用されます。
  ローカルに保存する設計でも、AIへの依頼は選んだサービスを経由します。

## インストール

Homebrewからインストールします。配布済みパッケージにはRustの開発環境は不要です。

```sh
brew install --cask ekeminusyou/lazypencil/lazypencil
lazypencil --version
```

更新には`brew upgrade --cask lazypencil`を使います。手動インストールやソースからの
ビルドは、[READMEのInstall](../README.md#install)を参照してください。

## まずAIなしで完成した画面を見る

新しい作業用ディレクトリに、ダッシュボードの`demo.pen`を用意します。
利用中のバージョンに応じて、次のどちらかを選んでください。

- **`lazypencil --help`に`demo`がある場合：** `lazypencil demo`を実行します。
  このコマンドは現在のソースで追加されたものです。
- **0.2.5など、`demo`のない配布版の場合：**
  [ダッシュボードの元ファイル](https://raw.githubusercontent.com/EkeMinusYou/lazypencil/main/examples/project-dashboard.pen)
  をダウンロードし、`demo.pen`という名前で保存します。

`demo`は既存ファイルを上書きしません。
別名なら`lazypencil demo my-dashboard.pen`を使えます。
保存先の親ディレクトリは事前に作成してください。
サンプルは実行ファイルに含まれ、コピー時のダウンロードやAI接続はありません。

用意した画面を`demo.png`に出力して開きます。

```sh
lazypencil render demo.pen --output demo.png
open demo.png
```

## AIで編集する方法を選ぶ

| 使い方 | 必要な準備 |
| --- | --- |
| 普段のAIクライアントから操作する | MCPを登録する。ACPアダプターは不要 |
| lazypencilのターミナル画面で会話する | 対応端末とCodexまたはClaudeのACPアダプターを用意する |

### 普段のAIクライアントから使う

CodexかClaude Codeなど、MCP対応のクライアントで使えます。利用するクライアントに
lazypencilを登録します。以下はどちらか一方を実行してください。

```sh
# Codex
codex mcp add lazypencil -- lazypencil mcp

# Claude Code
claude mcp add --scope user --transport stdio lazypencil -- lazypencil mcp
```

必要に応じてクライアントを再起動し、lazypencilのツールが認識されることを確認します。
`demo.pen`の絶対パスは、そのファイルのあるディレクトリで`pwd`を実行すると確認できます。
AIへの依頼では`/absolute/path/demo.pen`を実際のパスに置き換えてください。

```text
/absolute/path/demo.penを開いてください。
プロジェクトカードのアクセントカラーを青系に変更してください。
レイアウトを検証し、変更後の画面をPNGに出力して確認してください。
```

MCP経由では、AIへの接続にそのクライアントの認証を使います。
ターミナル画面を開く必要はありません。詳しい操作は[MCPリファレンス](mcp.md)にあります。

### ターミナル画面で使う

利用するAIのACPアダプターをインストールします。以下はどちらか一方を実行してください。

```sh
# Codex
npm install -g @agentclientprotocol/codex-acp@latest

# Claude
npm install -g @agentclientprotocol/claude-agent-acp@latest
```

Node.jsとnpmが必要です。認証は[アダプターごとの案内](agents.md#install-and-authenticate-an-adapter)
に従って行い、利用するAIの接続を診断します。

```sh
lazypencil doctor --agent codex
lazypencil tui demo.pen --agent codex
```

Claudeを使う場合は、両方の`codex`を`claude`に置き換えてください。
`doctor`はAIにプロンプトを送らず、モデルの利用枠や応答までは検証しません。

Tabで入力欄へ移動し、「サイドバーを少し狭くしてください」などと依頼します。
送信キーは画面下部に表示されます。F1で操作一覧を表示できます。
変更後は、同じファイルを`lazypencil render demo.pen --output demo.png`で画像にできます。

新規の画面を作る場合は、次のように起動します。

```sh
lazypencil tui my-screen.pen --create --agent codex
```

```text
メモアプリの設定画面を作ってください。
左側にサイドバー、右側にプロフィールと通知の設定を配置し、明るい配色にしてください。
```

`--create`は既存ファイルを上書きしません。次回からは`--create`を外して開きます。

## 別の題材も試す

[サンプルギャラリー](../examples/README.md)では、完成画像・編集可能なファイル・
修正用プロンプトをまとめています。

| サンプル | 試せる依頼 |
| --- | --- |
| プロジェクトダッシュボード | カードの配色を青系にし、カード間の余白を広げる |
| 日本語の設定画面 | 保存ボタンの文言を「変更を保存」にし、横幅を広げる |
| コンテンツページ | 内容を保ったまま、見出しと本文の文字サイズの差を大きくする |

設定画面にはローカル画像が付属するため、`.pen`だけでなく`examples/assets`も一緒に
取得してください。ギャラリーに依存ファイルを保持するコピー手順があります。

## うまく動かないとき

| 症状 | 確認すること |
| --- | --- |
| `lazypencil`が見つからない | Homebrewのインストール結果と`PATH`を確認する |
| `demo`が認識されない | 配布版には未搭載の場合がある。上記の元ファイルをダウンロードする |
| PNGは出せるが端末内に表示されない | 対応端末で試し、`doctor`のTerminal imagesの結果を確認する。[端末の制限](tui.md#initial-terminal-support)も参照 |
| MCPクライアントが起動できない | `lazypencil`の実行ファイルを絶対パスで登録し、クライアントを再起動する |
| ACPアダプターが見つからない | `doctor --agent codex`または`--agent claude`を実行し、実行ファイルと`PATH`を確認する |
| ログインや認証で止まる | 利用するAIランタイム側で認証し直す。[エージェント設定](agents.md)を参照 |
| 画像ファイルが見つからない | サンプルの相対パスと`assets`ディレクトリを保持しているか確認する |

ブラウザやGLSLを使うサンプルは、追加のChromiumランタイムが必要です。
このガイドのダッシュボードと、ギャラリーの3点には不要です。
Pencilとの完全な互換性は保証していません。[対応形式と制限](../README.md#format-support)を確認してください。

解決しない場合は、[Issue](https://github.com/EkeMinusYou/lazypencil/issues/new/choose)
から導入トラブルを報告できます。バージョン、macOS、端末、実行コマンドを添えると
状況を確認しやすくなります。認証情報や非公開の設計・会話内容は貼り付けないでください。
