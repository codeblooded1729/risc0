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

use risc0_zkvm::guest::env;

fn main() {
    let iterations: u32 = env::read();
    let answer = fibonacci(iterations);
    env::commit(&answer);
}

#[inline(never)]
#[no_mangle]
pub fn fibonacci(n: u32) -> u64 {
    let (mut a, mut b) = (0, 1);
    if n <= 1 {
        return n as u64;
    }
    let mut i = 2;
    while i <= n {
        let c = a + b;
        a = b;
        b = c;
        i += 1;
    }
    b
}
