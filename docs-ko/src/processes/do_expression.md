# `do`식

프로그램 전체에 걸쳐 *프로세스 문법*을 사용하는 것은 보통 바람직하지 않고, 대신 적절한 때에 필요한 만큼만 명령을 삽입하는 것이 최선이다. `do`식은 바로 이런 용도로 사용할 수 있는 문법이다. 실제 Par 프로그램에서는 **명시적으로 작성하는 대부분의 명령은 `do`식 안에 작성하게 될 것이다**. 또한 *식 문법*은 이미 배워서 익숙하므로, 식으로만 이루어진 문법에 `do`식으로 명령을 추가해보는 것이 좋은 시작점이 될 것이다.

`do`식은 키워드 `do`로 시작하고, 중괄호 안에 구분자 없이 명령을 나열한 뒤 그 뒤에 키워드 `in`을 적고 결과 식을 작성하면 된다.

`do`식은 우선 중괄호 안의 명령을 실행하고, 그 뒤에 `in` 다음의 식으로 평가된다.

```par
def MyName: String = do { } in "Michal"
```

위의 `do`식에는 명령이 없으므로, 식의 결과는 그대로 `"Michal"`이 된다.

## `let`문

실제로 채널을 조작하는 명령은 아니지만 프로세스 내에 작성할 수 있는 것이 또 있다. 바로 `let`문이다. [`let`/`in`식](../structure/let_expressions.md)과 같이 변수에 값을 대입하는 기능을 하지만, `let`문에는 `in` 키워드가 없다는 차이점이 있다. 또한 프로세스 내부이기 때문에 여러 `let`을 연속으로 작성할 수 있다.

```par
module Main

import {
  @core/Nat
  @core/String
}

dec DisplayPlusEquation : [Nat, Nat] String
def DisplayPlusEquation = [a, b] do {
  let c = a + b
  let a = `#{a}`
  let b = `#{b}`
  let c = `#{c}`
} in `${a}+${b}=${c}`

def Test = DisplayPlusEquation(3, 4)  // = "3+4=7"
```

사실 이 방법이 **식 하나에서 여러 변수를 정의하는 가장 자연스러운 방법**이다.
