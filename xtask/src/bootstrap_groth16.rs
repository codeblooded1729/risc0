// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{
    fs,
    fs::{read_to_string, File},
    io::Cursor,
    path::Path,
    process::Command,
};

use clap::Parser;
use hex::FromHex;
use regex::Regex;
use risc0_seal_to_json::to_json;
use risc0_zkvm::{
    get_prover_server,
    recursion::{identity_p254, lift},
    sha::{Digest, Digestible},
    ExecutorEnv, ExecutorImpl, Groth16Receipt, Groth16Seal, InnerReceipt, ProverOpts, Receipt,
    VerifierContext, ALLOWED_IDS_ROOT,
};
use risc0_zkvm_methods::{multi_test::MultiTestSpec, MULTI_TEST_ELF, MULTI_TEST_ID};
use tempfile::tempdir;
use tracing_subscriber::fmt::format;

#[derive(Parser)]
pub struct BootstrapGroth16;

const SOL_HEADER: &str = r#"// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

// This file is automatically generated by:
// cargo xtask bootstrap-groth16

"#;

const SOLIDITY_GROTH16_VERIFIER_PATH: &str =
    "bonsai/ethereum/contracts/groth16/Groth16Verifier.sol";
const SOLIDITY_CONTROL_ID_PATH: &str = "bonsai/ethereum/contracts/groth16/ControlID.sol";
const SOLIDITY_TEST_RECEIPT_PATH: &str = "bonsai/ethereum/contracts/test/TestReceipt.sol";
const RUST_GROTH16_VERIFIER_PATH: &str = "risc0/zkvm/src/host/groth16.rs";

impl BootstrapGroth16 {
    pub fn run(&self) {
        bootstrap_verifying_key();
        bootstrap_control_id();
        bootstrap_test_receipt();
    }
}

fn bootstrap_verifying_key() {
    let solidity_code = read_to_string(SOLIDITY_GROTH16_VERIFIER_PATH).expect(&format!(
        "failed to read the Solidity verifier from {}",
        SOLIDITY_GROTH16_VERIFIER_PATH
    ));
    let mut rust_code = read_to_string(RUST_GROTH16_VERIFIER_PATH).expect(&format!(
        "failed to read groth16.rs from {}",
        RUST_GROTH16_VERIFIER_PATH
    ));

    let solidity_constants = [
        "alphax", "alphay", "betax1", "betax2", "betay1", "betay2", "gammax1", "gammax2",
        "gammay1", "gammay2", "deltax1", "deltax2", "deltay1", "deltay2", "IC0x", "IC0y", "IC1x",
        "IC1y", "IC2x", "IC2y", "IC3x", "IC3y", "IC4x", "IC4y",
    ];

    let rust_constants = [
        "ALPHA_X", "ALPHA_Y", "BETA_X1", "BETA_X2", "BETA_Y1", "BETA_Y2", "GAMMA_X1", "GAMMA_X2",
        "GAMMA_Y1", "GAMMA_Y2", "DELTA_X1", "DELTA_X2", "DELTA_Y1", "DELTA_Y2", "IC0_X", "IC0_Y",
        "IC1_X", "IC1_Y", "IC2_X", "IC2_Y", "IC3_X", "IC3_Y", "IC4_X", "IC4_Y",
    ];

    for (i, constant) in solidity_constants.into_iter().enumerate() {
        let re = Regex::new(&format!(r"uint256 constant\s+{}\s*=\s*(\d+);", constant)).unwrap();
        if let Some(caps) = re.captures(&solidity_code) {
            let rust_re = Regex::new(&format!(
                "const {}: &str =[\\r\\n\\s]*\"\\d+\";",
                rust_constants[i]
            ))
            .unwrap();
            rust_code = rust_re
                .replace(
                    &rust_code,
                    &format!("const {}: &str = \"{}\";", rust_constants[i], &caps[1]),
                )
                .to_string();
        } else {
            println!("{} not found", constant);
        }
    }

    fs::write(RUST_GROTH16_VERIFIER_PATH, rust_code).expect(&format!(
        "failed to save changes to {}",
        RUST_GROTH16_VERIFIER_PATH
    ));

    // Use rustfmt to format the file.
    Command::new("rustfmt")
        .arg(RUST_GROTH16_VERIFIER_PATH)
        .status()
        .expect("failed to format {RUST_GROTH16_VERIFIER_PATH}");
}

fn bootstrap_control_id() {
    const LIB_HEADER: &str = r#"pragma solidity ^0.8.9;

 library ControlID {
"#;
    let (control_id_0, control_id_1) = split_digest(Digest::from_hex(ALLOWED_IDS_ROOT).unwrap());
    let control_id_0 = format!("uint256 public constant CONTROL_ID_0 = {control_id_0};");
    let control_id_1 = format!("uint256 public constant CONTROL_ID_1 = {control_id_1};");
    let content = &format!("{SOL_HEADER}{LIB_HEADER}\n{control_id_0}\n{control_id_1}\n}}");
    fs::write(SOLIDITY_CONTROL_ID_PATH, content).expect(&format!(
        "failed to save changes to {}",
        SOLIDITY_CONTROL_ID_PATH
    ));

    // Use forge fmt to format the file.
    Command::new("forge")
        .arg("fmt")
        .arg(SOLIDITY_CONTROL_ID_PATH)
        .status()
        .expect("failed to format {SOLIDITY_CONTROL_ID_PATH}");
}

fn bootstrap_test_receipt() {
    const LIB_HEADER: &str = r#"pragma solidity ^0.8.13;

 library TestReceipt {
"#;
    let (receipt, image_id) = generate_receipt();
    let seal = hex::encode(receipt.inner.groth16().unwrap().seal.clone());
    let post_digest = format!(
        "0x{}",
        hex::encode(receipt.get_claim().unwrap().post.digest().as_bytes())
    );
    let image_id = format!("0x{}", hex::encode(image_id.as_bytes()));
    let journal = hex::encode(receipt.journal.bytes);

    let seal = format!("bytes public constant SEAL = hex\"{seal}\";");
    let post_digest = format!("bytes32 public constant POST_DIGEST = bytes32({seal});");
    let journal = format!("bytes public constant JOURNAL = hex\"{journal}\";");
    let image_id = format!("bytes32 public constant IMAGE_ID = bytes32({image_id});");

    let content =
        &format!("{SOL_HEADER}{LIB_HEADER}\n{seal};\n{post_digest};\n{journal};\n{image_id};\n}}");
    fs::write(SOLIDITY_TEST_RECEIPT_PATH, content).expect(&format!(
        "failed to save changes to {}",
        SOLIDITY_TEST_RECEIPT_PATH
    ));

    // Use forge fmt to format the file.
    Command::new("forge")
        .arg("fmt")
        .arg(SOLIDITY_TEST_RECEIPT_PATH)
        .status()
        .expect("failed to format {SOLIDITY_TEST_RECEIPT_PATH}");
}

// Splits the digest in half returning the halves as big endiand
fn split_digest(d: Digest) -> (String, String) {
    let big_endian: Vec<u8> = d.as_bytes().to_vec().iter().rev().cloned().collect();
    let middle = big_endian.len() / 2;
    let (control_id_0, control_id_1) = big_endian.split_at(middle);
    (
        format!("0x{}", hex::encode(control_id_0)),
        format!("0x{}", hex::encode(control_id_1)),
    )
}

// Return a Groth16 receipt and the imageID used to generate the proof.
// Requires running Docker on an x86 architecture.
fn generate_receipt() -> (Receipt, Digest) {
    let tmp_dir = tempdir().expect("Failed to create tmpdir");
    let work_dir = std::env::var("RISC0_WORK_DIR");
    let work_dir = work_dir
        .as_ref()
        .map(|x| Path::new(x))
        .unwrap_or(tmp_dir.path());

    let env = ExecutorEnv::builder()
        .write(&MultiTestSpec::BusyLoop { cycles: 0 })
        .unwrap()
        .build()
        .unwrap();

    tracing::info!("execute");
    let mut exec = ExecutorImpl::from_elf(env, MULTI_TEST_ELF).unwrap();
    let session = exec.run().unwrap();
    let segments = session.resolve().unwrap();
    assert_eq!(segments.len(), 1);

    tracing::info!("prove");
    let opts = ProverOpts::default();
    let ctx = VerifierContext::default();
    let prover = get_prover_server(&opts).unwrap();
    let segment_receipt = prover.prove_segment(&ctx, &segments[0]).unwrap();

    tracing::info!("lift");
    let lift_receipt = lift(&segment_receipt).unwrap();
    lift_receipt.verify_integrity().unwrap();

    tracing::info!("identity_p254");
    let ident_receipt = identity_p254(&lift_receipt).unwrap();
    let seal_bytes = ident_receipt.get_seal_bytes();

    std::fs::write(work_dir.join("seal.r0"), &seal_bytes).unwrap();

    let journal = session.journal.unwrap().bytes;
    let succinct_receipt = Receipt::new(InnerReceipt::Succinct(ident_receipt), journal.clone());

    tracing::info!("seal-to-json");
    let seal_path = work_dir.join("input.json");
    let snark_path = work_dir.join("output.json");
    let seal_json = File::create(&seal_path).unwrap();
    let mut seal_reader = Cursor::new(&seal_bytes);
    to_json(&mut seal_reader, &seal_json).unwrap();

    tracing::info!("risc0-groth16-prover");
    let status = Command::new("docker")
        .arg("run")
        .arg("--rm")
        .arg("-v")
        .arg(&format!("{}:/mnt", work_dir.to_string_lossy()))
        .arg("risc0-groth16-prover")
        .status()
        .unwrap();
    if !status.success() {
        panic!("docker returned failure exit code: {:?}", status.code());
    }

    let snark_str = read_to_string(snark_path).unwrap();
    tracing::info!("{snark_str}");
    let snark_str = format!("[{snark_str}]"); // make the output valid json
    let raw_proof: (Vec<String>, Vec<Vec<String>>, Vec<String>, Vec<String>) =
        serde_json::from_str(&snark_str).unwrap();

    tracing::info!("decode a");
    let a: Result<Vec<Vec<u8>>, hex::FromHexError> = raw_proof
        .0
        .into_iter()
        .map(|x| hex::decode(&x[2..]))
        .collect();
    let a = a.expect("Failed to decode snark 'a' values");

    tracing::info!("decode b");
    let b: Result<Vec<Vec<Vec<u8>>>, hex::FromHexError> = raw_proof
        .1
        .into_iter()
        .map(|inner| {
            inner
                .into_iter()
                .map(|x| hex::decode(&x[2..]))
                .collect::<Result<Vec<Vec<u8>>, hex::FromHexError>>()
        })
        .collect();
    let b = b.expect("Failed to decode snark 'b' values");

    tracing::info!("decode c");
    let c: Result<Vec<Vec<u8>>, hex::FromHexError> = raw_proof
        .2
        .into_iter()
        .map(|x| hex::decode(&x[2..]))
        .collect();
    let c = c.expect("Failed to decode snark 'c' values");

    tracing::info!("Groth16Seal");
    let groth16_seal = Groth16Seal { a, b, c };
    let receipt = Receipt::new(
        InnerReceipt::Groth16(Groth16Receipt {
            seal: groth16_seal.to_vec(),
            claim: succinct_receipt.get_claim().unwrap(),
        }),
        journal,
    );
    let image_id = Digest::from(MULTI_TEST_ID);
    (receipt, image_id)
}
