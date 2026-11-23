pub(crate) struct Asset {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub(crate) const RUNTIME_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip";
pub(crate) const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin";
pub(crate) const ARCHIVE: Asset = Asset {
    name: "runtime.zip",
    size: 8_928_640,
    sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
};
pub(crate) const MODEL: Asset = Asset {
    name: "ggml-base.bin",
    size: 147_951_465,
    sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
};
pub(crate) const RUNTIME: &[Asset] = &[
    Asset {
        name: "ggml-base.dll",
        size: 693248,
        sha256: "bede0d6b2d387236ecd2b5dd676d51c8984b5b6e0ae461a12174e9c01345c5f0",
    },
    Asset {
        name: "ggml-cpu-alderlake.dll",
        size: 1009664,
        sha256: "d8f5c56ea490394eeae8b271650f2647aecd039a542d6df9e0140726e62a4603",
    },
    Asset {
        name: "ggml-cpu-cannonlake.dll",
        size: 1049600,
        sha256: "74127b40abd4f595d9db6af0fa313a02e18ff42fc228bfa05281e7c788dd4507",
    },
    Asset {
        name: "ggml-cpu-cascadelake.dll",
        size: 1061376,
        sha256: "4b11c427b844eeac87529af9a8d94b35aad2024666beb2a35e59fe3077e4b81e",
    },
    Asset {
        name: "ggml-cpu-haswell.dll",
        size: 1010688,
        sha256: "1306b1014c8118d27234d8ddc0e66ced1f8c8476bf85071595fd182ea9b7263b",
    },
    Asset {
        name: "ggml-cpu-icelake.dll",
        size: 1061376,
        sha256: "2b4dc4c66e7d8fe2807283816e9cf3b7f7d0556f59df34069871e37eab8aed40",
    },
    Asset {
        name: "ggml-cpu-sandybridge.dll",
        size: 1026560,
        sha256: "04edbdebc01a1d421062a960881a4f34d520d99693999518243f332406444c6e",
    },
    Asset {
        name: "ggml-cpu-skylakex.dll",
        size: 1049600,
        sha256: "1774714cf93621406ff35344b54715619a83242ae3e331e44d98a3b35ce801be",
    },
    Asset {
        name: "ggml-cpu-sse42.dll",
        size: 862720,
        sha256: "f060318772c4c2dbac312bbfff6ea700b85dbc502bb38585a7186d06d549301a",
    },
    Asset {
        name: "ggml-cpu-x64.dll",
        size: 866816,
        sha256: "4b4b05caba40d4309b5d710ef74c59ba190516b8489f27bdb1ba11ae33c6e9d3",
    },
    Asset {
        name: "ggml.dll",
        size: 62464,
        sha256: "31fe18616d872af9b6134b47a6dff8072ae4ecbe5df0273f4f8dc36766c9db8e",
    },
    Asset {
        name: "whisper-cli.exe",
        size: 479744,
        sha256: "331dba46d6427105d2b802cdbc7eae916ea1c5abf9b0d3b5cfe460d8db8e4366",
    },
    Asset {
        name: "whisper.dll",
        size: 1370112,
        sha256: "034396d75bb5720c1851674ff64a96192b58f6927b68d334ebd254ce6cad9674",
    },
];

pub(crate) const LICENSE: &str = "MIT License\n\nwhisper.cpp: Copyright (c) 2023-2026 The ggml authors\nWhisper model: Copyright (c) 2022 OpenAI\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.\n";
