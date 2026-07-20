[English](design.md) | **日本語**

# 設計

## なぜ作るか

個々のプロンプトインジェクション・ジェイルブレイクベンチマークを走らせるツールは既に存在する．本基盤の存在理由はそこではなく，**同一の測定器・同一の統計処理・同一の実行条件を全ベンチマークに強制すること**にある．既存のジェイルブレイク／プロンプトインジェクション・ベンチマークには構造的な欠陥があり，以下はいずれも推測ではなく，上流リポジトリを実際に取得して実測した結果に基づく．

### 測定器（judge）が信用できない

LLM-judge の再現率は 0.06〜0.65 と幅があり，judge の信頼性を数値で開示しないベンチマークが多い．**HarmBench 分類器の偽陽性率 FPR = 0.268 を独立に再現**した．人手多数決による「真の ASR」36.7% に対し，同分類器は 49.0%（1.34 倍に水増し）と報告する．同一の正解集合上の 4 分類器のうち，`harmbench_cf` だけが外れ値で，他 3 種の judge とは 77〜78% しか一致しない（他 3 種は互いに 89〜93% 一致する）．人手アノテータ間の Cohen's kappa は 0.809 / 0.826 / 0.886 で，これが測定精度の上限である．この正解集合は JailbreakBench 論文が用いたものと同一で，本計測はその値を独立に再現している．

### AdvBench の「二重梱包」

HarmBench は AdvBench を再梱包しており，520 件のプロンプトテキストは完全一致する．しかし再梱包側は，原典 `llm-attacks` が持つ **`target` 列**（`"Sure, here is ..."`；GCG の最適化目標で，拒否文字列マッチ ASR が依拠する列）を落としている．同じテキストでも，どちらの写しを読むかで実行可能な測定が変わる．本基盤は原典 `llm-attacks` を AdvBench の正とする．

### ベンチマークは互いに独立でない

完全一致による実測重複は JBB ∩ AdvBench = **11**，JBB ∩ HarmBench = **9**，AdvBench ∩ HarmBench = **0** である．「3 つのベンチが一致した」が同じ設問を複数回数えているだけのことがある．自己申告の `Source` 列は当てにならず，`AdvBench` 申告 18 件のうち実際に一致するのは 11 件である．

### 「判定不能」が第 3 の状態として実在する

HarmBench 分類器は yes/no 以外で `-1` を，StrongREJECT v1 はパース失敗時に `NaN` を返す．0 に潰せば ASR は下振れ，成功に潰せば上振れするが，どちらの潰しも報告されない．**判定は三値**である．

### 「StrongREJECT の judge」が 2 つある

原典 `alexandrasouly/strongreject`（deprecated, v1）と後継 `dsbowen/strong_reject`（v2）で**ルーブリックのプロンプトが異なる**．採点式は代数的に同一で，差は完全にプロンプト側にある．「StrongREJECT スコア 0.42」と書かれた 2 論文が，別の測定器の数字である可能性がある．

## 設計原則

- **三値判定** — 判定は `Success | Failure | Undecidable { reason }` である．判定不能を成功／失敗に黙って潰さず，二値への還元は集計時の明示的な選択とし，潰した件数を必ず併記する．
- **CaseId と ContentKey の分離** — 同一性（identity）は provenance `(repo, commit, path, row)` を含む `CaseId` で決める（同一テキストでも出自が違えば別 Case）．重複検出（dedup）は出自を含めない内容フィンガープリント `ContentKey = hash(normalize(prompt), normalize(context))` で行う．dedup キーは非独立性の**報告にのみ**使い，Case は統合しない．
- **union coverage（単一設定 ASR の否定）** — ある Case と攻撃バリアント集合 `V` について `coverage(case) = 1 iff ∃v∈V. verdict == Success` とする．変換は `Case` に焼き込まず，生成時に `(Case.prompt, AttackRef)` から最終プロンプトを導出し，`Case` は不変に保つ．
- **多試行 ASR + 信頼区間** — 1 / 10 / 100 回の開示．単発 ASR（二項割合）は既定で **Wilson score 区間**，最悪ケース報告用に Clopper–Pearson をオプションとする．単純割合でない統計（多試行曲線・union coverage・judge 間差分）は **Case 単位のブートストラップ（percentile / BCa）** で出す．`Undecidable` は分母から除外し，件数を併記する．
- **network-free 既定** — 既定ビルドは HTTP を要さない．LLM プロバイダは cargo feature で gate し，既定は mock プロバイダである．正解データ（JBB `judge-comparison.csv`）を用いた judge 信頼性の再現は，LLM を 1 回も呼ばずに実行・テストできる．
- **ハイブリッド型モノレポ** — 自作コードは単一リポジトリに置く（core と adapter のバージョン不整合が静かに数値を動かすのを防ぐ）．上流ベンチの実体のみ `third_party/` に SHA 固定の submodule で持つ．
- **有害データを vendoring しない** — 設問データは submodule 参照とし実体を持たない．per-source ライセンスが未定義のもの（MaliciousInstruct・HarmfulQ・OpenAI System Card 等）があるため，submodule 参照が法的にも正しい．

補足: **環境種別（`EnvKind`）と adapter 種別は別物である**．`EnvKind`（`StaticPrompt` / `Emulated` / `RealExecutable`）はスコア比較の可否を決める科学的性質で第一級メタデータであり，adapter 種別（`native` / `sidecar`）は実装都合である．集計 API は「同一 `EnvKind` 内でのみ比較」を宣言で終わらせず型で強制し，測定器（`InstrumentRef`）を跨いだ ASR の単純平均も既定で禁止する．この規則は今や宣言に留まらず端から端まで強制され，かつ実証されている：静的プロンプトベンチの傍らに第 2 の環境種別（emulated なエージェント型ベンチ）が実在し，既定レポートは `EnvKind` ごとの ASR を並置して横断スカラを一切出さず，プール値は警告と一致度の指標を添えて開示する明示的 opt-in の裏にのみ置かれる．

## 責任ある利用 / データポリシー

本リポジトリは**防御的評価・安全性研究のための dual-use ツール**である．非目標を明示する：

- **リーダーボードを作らない**．出力は常に測定器・環境種別・信頼区間つきの条件付き数値であり，単一順位ではない．
- **新規攻撃手法を研究・生成しない**．Phase 1〜2 は既存ベンチの実行・測定が目的で，mutator は既存の公開手法の再現に限る．
- **安全性の認証・保証を与えない**．反証可能性の無い主張はマーケティング上の主張として扱う（自らにも適用する）．
- **有害コンテンツを同梱・再配布しない**．設問データは submodule 参照とし実体を持たず，copyright 系は生テキストでなくハッシュのみを扱う．

生成される攻撃プロンプトと有害応答（`Measurement.raw` 等に残りうる）は測定・監査の目的でのみ扱い，リポジトリ外に公開しない．**有害データおよび応答キャッシュはコミットしない**（`.cache/` / `results/` は gitignore 対象）．

## Phase 計画

- **Phase 1（完了）** — データセット型ベンチで縦串を通す．対象は AdvBench 520，HarmBench 400（contextual 100・copyright 100），StrongREJECT 313 + small 60（rubric v1・v2 両実装），JBB harmful 100 + benign 100 + judge-comparison 300．差別化点である「測定器の信頼性開示」「単一設定 ASR の否定（union coverage）」「多試行 ASR と信頼区間」「ベンチ間非独立性の自動検出」「過剰拒否」を，ここで実証する．4 ベンチが全て `StaticPrompt` のため，環境種別を跨ぐ比較は Phase 2 を待つ．
- **Phase 2（概ね完了）** — 環境型ベンチ（AgentDojo 等）を sidecar で取り込み，制御反転を実装する OpenAI 互換シムを通じて，全モデル呼び出しを単一の `pil-llm` 経路に集約する．実装済み: **制御反転**（シムがローカル OpenAI 互換エンドポイントを立て，sidecar がベンチの irreducible な本体をシムへ向けた client で走らせる — 温度・seed・cache・metadata が揃い，エージェント型ベンチ向けに tool-calling passthrough を持つ），**環境型ベンチ**（AgentDojo を `EnvKind::Emulated` として native-first で取り込み，シム経由でローカルのツール対応モデルに対しライブ実行できる），**`EnvKind` 跨ぎの報告**（`EnvKind` ごとの ASR を並置し，明示的な `--cross-env` opt-in 指定時のみプールスカラを警告および **Kendall の一致係数 W** とともに開示する — W は結論が judge／環境にどれだけ依存するかを定量化し，共通の測定器・ケースを持たない環境間では未定義として正しく報告される）．**環境種別を跨いだ横断比較がここで初めて成立する**．残り: fine-tuned judge（`qylu4156/strongreject-15k-v1`，logit 期待値式）．
- **Phase 3（候補）** — 適応型ベンチマークの標準形，マルチエージェント特有の創発的リスク（感染型 jailbreak・秘密結託・責任希薄化），AdvBench 近傍重複の埋め込みによる定量化．
