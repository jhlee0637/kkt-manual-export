"""difflib 이식을 검증할 정답 데이터를 Python 으로 만든다.

    python3 rust/crates/kkt-core/tests/data/gen_difflib_cases.py

작은 알파벳(중복이 많아 동률이 자주 생긴다)과 긴 시퀀스를 섞어, 같은 입력에 대한
get_matching_blocks / get_opcodes 결과를 JSON 으로 저장한다. Rust 테스트가 이를 그대로 대조한다.
"""
import json
import random
from difflib import SequenceMatcher
from pathlib import Path

random.seed(20261002)
cases = []


def add(a, b):
    sm = SequenceMatcher(None, a, b, autojunk=False)
    cases.append({"a": a, "b": b,
                  "blocks": [list(t) for t in sm.get_matching_blocks()],
                  "opcodes": [[tag, i1, i2, j1, j2] for tag, i1, i2, j1, j2 in sm.get_opcodes()]})


add([], []); add(["x"], []); add([], ["x"]); add(list("abc"), list("abc")); add(list("abc"), list("xyz"))
add(["t1", "t2", "p7", "d", "p8"], ["t1", "t22", "d", "d", "p8"])
for _ in range(400):
    alpha = random.choice(["ab", "abc", "abcd", "abcdefgh"])
    a = [random.choice(alpha) for _ in range(random.randint(0, 14))]
    b = list(a)
    for _ in range(random.randint(0, 5)):                       # 실제 사용처럼 a 를 조금 바꾼 b
        op = random.choice(["del", "ins", "sub"])
        if op == "del" and b:
            b.pop(random.randrange(len(b)))
        elif op == "ins":
            b.insert(random.randint(0, len(b)), random.choice(alpha))
        elif b:
            b[random.randrange(len(b))] = random.choice(alpha)
    add(a, b)
for _ in range(100):                                           # 완전히 무관한 두 시퀀스
    alpha = random.choice(["ab", "abc", "abcdef"])
    add([random.choice(alpha) for _ in range(random.randint(0, 20))],
        [random.choice(alpha) for _ in range(random.randint(0, 20))])
for _ in range(20):                                            # 긴 시퀀스
    a = [random.choice("abcdefghij") for _ in range(120)]
    b = list(a)
    for _ in range(15):
        b.insert(random.randint(0, len(b)), random.choice("abcdefghij"))
        b.pop(random.randrange(len(b)))
    add(a, b)

out = Path(__file__).with_name("difflib_cases.json")
out.write_text(json.dumps(cases, ensure_ascii=False) + "\n", encoding="utf-8")
print(f"{len(cases)} cases -> {out}")
