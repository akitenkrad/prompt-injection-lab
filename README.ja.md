<p align="center"><img src="docs/assets/hero.svg" width="100%"></p>

[English](README.md) | **日本語**

# prompt-injection-lab

既存のプロンプトインジェクション・ジェイルブレイクベンチマーク（AdvBench・HarmBench・StrongREJECT・JBB）を，任意の LLM に対して**同一の測定経路・同一の統計処理・同一の実行条件**で走らせ，比較可能な数値として測定する Rust 製の研究基盤である．個々のベンチマークを走らせるツールは既に存在する．本基盤の存在理由は，各ベンチマーク自身の報告を信用するのではなく，全ベンチマークに同一の測定器・集計・実行条件を強制することにある．

## できること

- **測定器（judge）の信頼性を開示する** — 正解ラベルに対して judge 自身の recall / FPR / precision / F1 を測り，HarmBench 分類器の偽陽性率 0.268 を独立に再現する．
- **単一設定 ASR を否定する** — 各 Case について攻撃バリアント集合上の union coverage を報告し，単一最良設定では報告しない．
- **多試行 ASR を信頼区間つきで報告する** — 既定は Wilson score 区間，必要に応じて Clopper–Pearson・ブートストラップ BCa を用い，常に判定不能（undecidable）件数を併記する．
- **ベンチマーク間の非独立性を検出する** — 内容フィンガープリントによりベンチマークを跨いだ重複設問を自動検出し，「3 ベンチが一致した」が同一設問を 3 回数えているだけの状態を防ぐ．
- **過剰拒否を測る** — 拒否率は benign プロンプト上の対の信号として扱い，単独の安全性スコアにはしない．
- **環境種別を跨いで比較する** — 4 つの静的プロンプトベンチに加え，エージェント型（ツール利用）ベンチを *emulated* 環境として取り込んだ．静的プロンプトベンチのインジェクション成功率と，エージェント環境のそれとを，単一の測定経路のもとで並置できる — 横断スカラに黙って潰すことはしない．

**ステータス:** Phase 1 は完了，Phase 2 — 制御反転・emulated なエージェント環境・環境種別を跨ぐ報告 — も概ね完了しており，残るは fine-tuned judge のみである．

## セットアップ

上流ベンチマークデータは `third_party/` の commit 固定した Git submodule として参照する（有害データはリポジトリに vendoring しない）ため，clone 時に submodule を取得する．

```bash
# submodule つきで clone
git clone --recurse-submodules git@github.com:akitenkrad/prompt-injection-lab.git

# 通常 clone 済みなら submodule を後から取得
git submodule update --init --recursive
```

既定ビルドはネットワーク非依存である（HTTP バックエンドは cargo feature で gate 済み）：

```bash
cargo build              # 既定 = network-free
cargo test --workspace
```

## ドキュメント

- [docs/design.ja.md](docs/design.ja.md) — 動機・根拠・設計原則・責任ある利用・Phase 計画．
- [docs/architecture.ja.md](docs/architecture.ja.md) — crate 構成と，環境型ベンチのために実装済みの制御反転設計．
- [docs/usage.ja.md](docs/usage.ja.md) — submodule・cargo feature つきのビルドとテスト・CLI（`reliability` / `run` / `report` / `agentdojo`）．

## ライセンス

MIT．`third_party/` 配下の上流 submodule は各々のライセンス（いずれも MIT）を保持する．
