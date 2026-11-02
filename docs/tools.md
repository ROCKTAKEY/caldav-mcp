# MCPツールの使い方

この文書のJSONはMCPツールに渡す引数です。サーバのURLやIDは、`list_calendars`、検索、詳細取得の結果を使ってください。

## 保存先と対象

カレンダーは、アカウントIDと絶対URLで指定します。同じUIDの予定が複数アカウントに存在しても、別の保存先として扱います。

```json
{
  "account": "personal",
  "url": "https://calendar.example.net/alice/tasks/"
}
```

リソースの `ref` はカレンダーと、その直下の `.ics` URLを持ちます。`revision` は取得時のETagです。ETagの引用符を含めて、そのまま渡します。

```json
{
  "ref": {
    "calendar": {
      "account": "personal",
      "url": "https://calendar.example.net/alice/tasks/"
    },
    "url": "https://calendar.example.net/alice/tasks/example.ics"
  },
  "revision": {"etag": "\"server-etag\""}
}
```

## タスクの作成と検索

`create_task` の例です。UIDはそのタスクに固有の値を指定します。同じ作成処理を確認・再試行するときは、UIDを変更しないでください。

```json
{
  "calendar": {
    "account": "personal",
    "url": "https://calendar.example.net/alice/tasks/"
  },
  "uid": "shopping-2026-11-09",
  "fields": {
    "summary": "お茶を買う",
    "due": {"kind": "date", "value": "2026-11-09"},
    "priority": 5,
    "categories": ["買い物"]
  }
}
```

`search_tasks` は `calendars` に複数のカレンダーを渡せます。`completed=false` は、状態が未設定のタスクも含めて未完了を探します。`period` を省略すると、日付を持たないタスクも検索します。期間指定はCalDAVのVTODO期間検索の意味に従い、期限だけの絞り込みではありません。`text` はタイトルの部分一致です。

`complete_task` と `reopen_task` には、取得した `ref`、`revision` と `scope` を渡します。

## 日時

予定とタスクの日時には、次のいずれかを使います。

```json
{"kind": "date", "value": "2026-11-09"}
```

```json
{"kind": "utc", "value": "2026-11-09T01:00:00Z"}
```

```json
{"kind": "zoned", "value": "2026-11-09T10:00:00", "timezone": "Asia/Tokyo"}
```

```json
{"kind": "floating", "value": "2026-11-09T10:00:00"}
```

夏時間の移行で存在しないローカル時刻は拒否します。二通りに解釈できる時刻は、意図するUTC時刻を指定してください。

検索と空き時間照会の `period` には、オフセット付きの日時を使います。`end` は `start` より後でなければなりません。

```json
{"start": "2026-11-09T00:00:00+09:00", "end": "2026-11-16T00:00:00+09:00"}
```

## 繰り返しと個別例外

`create_event` はタスクと同じ `calendar`・`uid`・`fields` の形式です。予定には `start` が必要です。終日の予定の `end` は終了日の翌日を指定します。

```json
{
  "summary": "週次レビュー",
  "start": {"kind": "zoned", "value": "2026-11-09T10:00:00", "timezone": "Asia/Tokyo"},
  "end": {"kind": "zoned", "value": "2026-11-09T11:00:00", "timezone": "Asia/Tokyo"},
  "recurrence_rule": "FREQ=WEEKLY;BYDAY=MO;COUNT=12",
  "exception_dates": [
    {"kind": "zoned", "value": "2026-11-23T10:00:00", "timezone": "Asia/Tokyo"}
  ]
}
```

`recurrence_dates` は追加開催日、`exception_dates` は除外日です。日時の種類は開始と一致させます。`recurrence_rule` はRFC 5545のRRULE文字列です。`COUNT` と `UNTIL` の併用や、0以下の `INTERVAL` は拒否します。

`search_events(expand=true)` で個別回を取得できます。`update_item` の `scope` には、結果の `recurrence_id` を指定します。

```json
{
  "kind": "occurrence",
  "recurrence_id": {"kind": "utc", "value": "2026-11-16T01:00:00Z"}
}
```

`changes` に新しい開始・終了時刻を指定します。`recurrence_id` は変更前の回の識別子であり、新しい開始時刻ではありません。シリーズの基本設定を編集する場合は `{"kind":"series"}` を使います。既存の個別例外はそのまま残ります。

`delete_item` の `scope=occurrence` はEXDATEを追加し、対象の個別例外を取り除きます。`scope=series` はリソース全体を削除します。

## 詳細フィールド

予定・タスク・日誌に共通する入力モデルを使い、コンポーネントごとの制約を更新後のデータ全体に適用します。例えば `due` はタスク専用で、`duration` と `end` / `due` は同時に指定できません。

- `description`、`location`、`categories`、`classification` は説明・場所・分類・公開区分です。
- `status`、`percent_complete`、`priority` は状態・進捗・優先度です。
- `related_to` は `{"uid":"parent-task","relationship":"PARENT"}` などの関連です。
- `attachments` は `{"uri":"https://example.org/agenda.pdf","media_type":"application/pdf"}` などのURI添付です。
- `alarms` は `{"description":"会議","trigger":"-PT15M","relative_to":"START"}` などの表示アラームです。絶対時刻には `trigger` にUTC日時オブジェクトを指定します。

`alarms`、`attachments`、`related_to`、追加日・除外日などの配列を指定すると、その項目全体が置換されます。未指定なら保持します。`get_item` には元のiCalendarも含まれるため、編集前の詳細を確認できます。

## カレンダー・共有・空き時間

`create_calendar` は `spec` に `account`、`name` と任意の `components`、`description`、`timezone` を指定します。`update_calendar` は `changes` に `name`、`description`、`color`、`timezone` を指定します。カレンダー自体の削除は含まれる全データを削除する操作です。

`create_share` は `calendar`、相手のRadicaleユーザー名 `user`、共有先 `path`、`access` を指定します。`access` は `read`、`read_write`、`free_busy` のいずれかです。

共有の受け入れは、相手のアカウントで次のように `update_share` を呼びます。

```json
{"account":"work", "path":"/bob/team/", "enabled":true, "hidden":false}
```

所有者が `access` を変更すると共有権限が変わります。相手は `enabled=false` で無効化できます。`delete_share` は所有者が共有を解除する操作です。

`free_busy` は `calendars` と `period` を受け取ります。各カレンダーの `busy`、`free` と、全体の `free` を返します。勤務時間や候補時刻の順位付けは行いません。`free_busy` 権限だけの共有でも、予定の詳細を取得せず照会します。
