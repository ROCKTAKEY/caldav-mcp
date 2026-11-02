# caldav-mcp

ローカルLLMから、複数アカウントの予定・タスク・共有カレンダーを操作するMCPサーバです。CalDAVへ直接アクセスし、ローカルDB、キャッシュ、バックグラウンド同期を持ちません。接続方式はstdioです。

例えば、次のような依頼をMCPホスト上のLLMから実行できます。

```text
「個人用と仕事用の来週の予定を調べて」
  list_calendars → search_events

「買い物タスクを完了にして」
  search_tasks → get_item → complete_task

「来週の定例会だけ、1時間遅らせて」
  search_events → get_item → update_item(scope=occurrence)

「自分のカレンダーをbobに読み取り専用で共有して」
  create_share → bobのアカウントでupdate_share(enabled=true, hidden=false)
```

## 導入

Python 3.12以降とuvを使用します。リポジトリのルートで実行します。

```sh
uv sync --frozen --no-dev
cp examples/config.toml config.toml
```

[設定例](examples/config.toml)のURL、ユーザー名、アカウントIDを変更します。パスワードは、MCPホストから子プロセスへ引き継ぐ環境変数に設定します。`password_env` は値ではなく環境変数名です。認証情報をツールの引数としてLLMに渡す必要はありません。

```sh
uv run --frozen --no-dev caldav-mcp --config /absolute/path/to/config.toml
```

これは対話式CLIではなく、標準入力からMCPメッセージを受け取るプロセスです。通常はMCPホストから起動します。`--help` で起動引数を確認できます。

MCPホストの設定例です。コマンドや設定ファイルのパスは実際の配置に合わせてください。

```json
{
  "mcpServers": {
    "caldav": {
      "command": "/absolute/path/to/caldav-mcp/.venv/bin/python",
      "args": ["-m", "caldav_mcp", "--config", "/absolute/path/to/config.toml"]
    }
  }
}
```

## アカウントとカレンダー

`accounts` に複数の接続設定を列挙します。同じRadicaleの別ユーザーと、別サーバのアカウントをどちらも登録できます。

- `id` はMCP操作で使う一意の識別子です。
- `url` はリダイレクトを必要としないCalDAVの接続URLです。通常は末尾を `/` にします。
- `username` と `password_env` はBasic認証用です。Bearer認証を使う場合は、代わりに `bearer_token_env` を指定します。
- `calendars` には、自動検出されない共有カレンダーの絶対URLを追加できます。
- `default_calendar` はLLMに提示する既定の保存先候補です。書き込み時には常に `calendar` を明示します。
- `sharing_url` はRadicaleの共有APIのURLです。共有管理を使う場合に設定します。
- `timeout_seconds` はHTTPタイムアウトで、既定値は30秒です。

アカウント内のURLは同一オリジンに限定します。別ホストのカレンダーは別アカウントとして設定してください。TLS証明書検証は有効です。

`timezone` は利用者のタイムゾーンです。現在日時の表示、新規カレンダーの既定タイムゾーン、検索時に指定するfloating timeの解釈に使用します。個々の予定・タスクでは、日付のみ・UTC・タイムゾーン付き・floatingを明示的に区別します。

## 利用できる操作

23個のツールを公開します。引数のJSON Schemaは `tools/list` から取得できます。操作例と入力形式は[ツールの使い方](docs/tools.md)を参照してください。

- アカウント一覧、カレンダー一覧・詳細・作成・属性変更・削除。
- 複数カレンダーを横断する予定・タスク・日誌の検索と詳細取得。
- 予定・タスク・日誌の作成・更新・削除、タスクの完了・再開。
- 終日・タイムゾーン・繰り返し規則・追加日・除外日・個別回の変更と削除。
- タスクの期限・進捗・優先度・分類・親子関係、アラーム、URI添付。
- サーバによる空き時間照会と、選択したカレンダーすべてで空いている時間の取得。
- Radicaleユーザー間の共有作成、受け入れ、権限変更、非表示・無効化、解除。

アラームはDISPLAY形式の相対時刻と絶対UTC時刻を設定できます。添付はURIを記録します。実通知の配信や添付ファイルのアップロードは行いません。既存の参加者、未編集のアラーム、独自プロパティは保持します。

## Radicaleの共有設定

通常の読み書きはCalDAVを使います。共有相手の追加・権限変更・解除は、Radicale 3.7以降の共有専用APIを使います。サーバ側で利用者への共有作成を許可する必要があります。設定例は次のとおりです。

```ini
[sharing]
type = files
collection_by_map = True
permit_create_map = True
```

`permit_create_map` を全体で有効にせず、rights設定の `M` 権限で対象を限定することもできます。既存サーバの設定はこのMCPサーバから変更しません。[Radicaleの共有設定](https://radicale.org/v3.html#sharing)を参照してください。

共有を作成しただけでは相手側で有効になりません。相手のアカウントから `list_shares` で確認し、`update_share` に `enabled=true` と `hidden=false` を渡すと、カレンダー一覧に現れます。共有先パスは `/bob/team/` のようなRadicale内部のパスです。

共有APIがないバージョンや共有機能が無効な環境では、共有管理ツールは `unsupported` を返します。rightsファイルで既に許可されているカレンダーは、URLを設定して読み書きできます。トークンによる公開共有、グループ共有の作成、rightsファイルの編集は提供しません。

## 更新・検索の意味

更新・削除には、取得結果の `ref` と `revision` を使います。ETagが変わっていれば `conflict` となるため、改めて取得して変更内容を判断してください。作成に使うUIDは再試行時も同じ値を使います。

更新の引数を省略すると、その項目は保持されます。`null` は削除を意味します。配列項目を指定するとその項目全体を置換します。

`series` は基本コンポーネントを編集し、既存の個別例外を保持します。`occurrence` は元の `recurrence_id` で特定した1回を編集します。時間を移動するときは終了時刻・期限も明示的に調整してください。

検索・空き時間の期間評価は接続先サーバが行います。Radicale 3.9.0のfree-busy処理は日付のみ・floatingの値をUTCとして扱います。特定の地域の時刻に合わせた空き時間が必要な予定には、タイムゾーン付き日時を指定してください。

「この回以降」の編集は公開しません。Radicale 3.9.0の実サーバ試験では、`RANGE=THISANDFUTURE` を保存しても後続回の時刻に反映されないことを確認しています。シリーズの分割による代替処理も提供しません。

検索結果の `complete`、`errors`、`truncated` を確認してください。一部の接続失敗は空のカレンダーとは区別されます。空き時間照会で一部のカレンダーを取得できなければ、全体の `free` は `null` です。件数超過時は期間や検索条件を絞ってください。検索件数は最大500件、HTTP応答は展開後16 MiBまでです。

通信断で書き込み結果を確定できない場合は `outcome_unknown`、書き込み成功後の再取得だけに失敗した場合は `readback_failed` を返します。対象を読み直してから次の操作を判断してください。

## 対応確認と開発

Radicale 3.9.0を一時起動し、2ユーザーで共有・予定・タスク・空き時間照会を検証しています。MCPのstdio接続についても公式SDKのクライアントで初期化と操作を試験します。実際のローカルLLMによるツール選択精度と、利用者の稼働中サーバとの接続は別途確認が必要です。

```sh
uv sync --frozen
uv run --frozen pytest
uv run --frozen ruff check src tests
uv run --frozen ruff format --check src tests
uv run --frozen mypy src
uv build
```

テストは一時ディレクトリ内のRadicaleのみを操作します。利用者の設定や実サーバは読み込みません。

[設計](docs/design.md)では、型付き入力、更新の状態、機能ごとの責務を説明しています。ライセンスは[AGPL-3.0](LICENSE)です。
