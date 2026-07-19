//! Phase 1 CLI の canned judge（DESIGN §8.2 / §8.3 / IMPLEMENTATION_PLAN M10）．
//!
//! LLM 生成判定器（HarmBench cls・StrongREJECT rubric）は本来 judge モデルの応答を要するが，
//! Phase 1 の CLI は **ネットワーク非依存**を既定にするため，判定器の生出力を決定論的に合成する
//! canned judge を配線する（§8.2 の `Judge` 抽象が生成経路の注入を許す）．実 LLM judge の結線は
//! Phase 2（§8.3）．生成プロバイダ（`--provider`）とは独立で，ollama 生成でも judge はここを通る．
//!
//! canned judge は入力（system + user）の決定論ハッシュから出力を組む．user には被判定応答が
//! 埋め込まれる（rubric/cls とも）ため，attempt / attack バリアントが違えば判定も変わり，
//! union coverage・多試行 ASR 曲線・測定器間の不一致に**変化**が現れる（DESIGN §2.2 / §11.3 / §3.7）．

use pil_metrics::instrument::JudgeError;

/// judge クロージャの型（`Judge` の blanket impl を満たす関数ポインタ）．
pub type CannedJudge = fn(Option<&str>, &str) -> Result<String, JudgeError>;

/// FNV-1a（64bit）．外部依存を増やさず決定論ハッシュを得る．
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// HarmBench cls 用 canned judge（`yes` / `no`）．
///
/// 入力ハッシュの剰余で約 2/3 を `yes`（Success）に割り当てる．yes/no 以外は返さないため，
/// `ClassifierOutOfVocab` は起きない（変化は Success/Failure に集中する）．
pub fn canned_cls_judge(_system: Option<&str>, user: &str) -> Result<String, JudgeError> {
    let h = fnv1a(user);
    Ok(if h % 3 == 0 {
        "no".to_string()
    } else {
        "yes".to_string()
    })
}

/// StrongREJECT rubric 用 canned judge（`#scores` ブロック）．
///
/// `1.b`（refusal 0/1）・`2.b`（convincing 1-5）・`3.b`（specific 1-5）を決定論的に埋める．
/// v1/v2 いずれのパーサでも解釈できる形式で出力する（refusal>0 で Failure，§5.3）．
pub fn canned_rubric_judge(_system: Option<&str>, user: &str) -> Result<String, JudgeError> {
    let h = fnv1a(user);
    let refusal = u32::from(h % 4 == 0); // 約 1/4 を拒否（Failure）
    let convincing = 1 + ((h / 7) % 5) as u32; // 1..=5
    let specific = 1 + ((h / 13) % 5) as u32; // 1..=5
    Ok(format!(
        "#scores\n1.b {refusal}\n2.b {convincing}\n3.b {specific}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cls_judge_is_deterministic_and_in_vocab() {
        let a = canned_cls_judge(None, "abc").unwrap();
        let b = canned_cls_judge(None, "abc").unwrap();
        assert_eq!(a, b);
        assert!(a == "yes" || a == "no");
    }

    #[test]
    fn rubric_judge_emits_parseable_scores() {
        let out = canned_rubric_judge(None, "some response text").unwrap();
        assert!(out.contains("#scores"));
        assert!(out.contains("1.b "));
        assert!(out.contains("2.b "));
        assert!(out.contains("3.b "));
    }

    #[test]
    fn different_inputs_can_differ() {
        // 応答が違えば判定に変化が出る余地があること（union / 多試行の意味づけ）
        let outs: Vec<String> = (0..20)
            .map(|i| canned_cls_judge(None, &format!("resp {i}")).unwrap())
            .collect();
        assert!(outs.iter().any(|o| o == "yes"));
        assert!(outs.iter().any(|o| o == "no"));
    }
}
