# 수신 명령이 빛을 발할 때

이전 절에서는 `value[variable]` 수신 명령을 살펴 보았지만, 얼핏 보기에는 굳이 사용할 필요가 적어 보였다. 이제 이 명령이 진정 빛을 발하는, 무한 수열에서 값을 추출하는 용례를 살펴 보자.

앞에서 다룬 `Sequence<a>` 타입을 다시 사용할 것이다. 아래에서 정의를 다시 확인할 수 있다.

```par
type Sequence<a> = iterative choice {
  .close => !,
  .next => (a) self,
}
```

`Sequence`는 무제한 개수의 값을 하나씩 생성한 뒤 최종적으로 닫을 수 있는 타입이다.

## 피보나치 수열을 값으로 구현

피보나치 수열을 구현하는 것으로 시작해 보자.

```par
dec Fibonacci : Sequence<Nat>
def Fibonacci =
  let (a) b = (0) 1
  in begin case {
    .close => !
    .next =>
      let (a) b = (b) {a + b}
      in (a) loop
  }
```

이 수열의 내부 상태는 순서쌍 `(a) b`이다. `.next`가 한 번 선택될 때마다 `a`를 반환한 뒤 순서쌍을 갱신하며, [반복 타입](../../types/iterative.md)에서 이미 확인했듯이 쌍대재귀 반복의 깔끔하고 미려한 용법에 해당한다.

지금까지는 좋다. 그런데 실제로 사용하려면?

## 목표: 피보나치 수열의 처음 30개 항 출력

피보나치 수열의 처음 30개 항을 터미널에 출력하고 싶다고 해 보자.

목표를 달성하려면,

1. 특정한 동작을 30회 반복하고,
2. 매 회마다 수열에서 값을 추출하여,
3. 문자열로 바꾸고,
4. 표준 출력으로 보내야 한다.

어떻게든 **정확히 30번 반복**하는 코드를 구현해야 한다. 그런데 Par에서는 **재귀 타입만 루프가 가능하다**. `Nat`에는 자체적인 재귀 기능이 없으므로, 자연수 30을 [재귀](../../types/recursive.md) 값으로 변환해야 한다.

다행히 이 기능을 하는 내장 헬퍼가 있다. `Nat.Repeat`라는 이름이 있고, 타입은 다음과 같다.

```par
dec Nat.Repeat : [Nat] recursive either {
  .end!,
  .step self,
}
```

`Nat.Repeat(30)`을 호출하면 정확히 30회 반복할 수 있는 재귀 값을 얻을 수 있다. 바로 이 값을 사용하면 된다.

절반을 완성했다. 이제 값을 출력해 보자.

## `Console` 출력

Par에는 표준 출력을 다룰 수 있는 정의가 내장되어 있다.

```par
def Console.Open : Console
```

`Console` 타입 자체는 다음과 같이 정의되어 있다.

```par
type Console = iterative choice {
  .close => !,
  .print(String) => self,
}
```

문자열을 전달받아 터미널 화면으로 보내는 일종의 '싱크'(sink)로 생각하면 된다. `String.Builder`와 비슷하지만 문자열이 바로 콘솔로 전송된다.

코드를 완성해 보자!

```par
module Main

import {
  @basic/Console
  @core/Nat
}

def Program: ! = do {
  let console = Console.Open
  let fib = Fibonacci

  Nat.Repeat(30).begin.case {
    .end! => {}
    .step remaining => {
      fib.next[n]
      console.print(`#{n}`)
      remaining.loop
    }
  }

  fib.close
  console.close
} in !
```

코드를 해설하면 다음과 같다.

- `Nat.Repeat(30)`이 `.begin`과 `.case` 명령의 주어가 된다.
- 매 `.step`마다 `fib.next[n]` 명령을 사용해 `fib` 수열에서 원소를 추출한다.
- 이 값을 템플릿 문자열을 사용해 문자열로 변환하고 콘솔에 출력한다.
- 마지막으로 `loop`를 한다.

*수신* 명령이 진정으로 빛을 발하는 곳이 바로 `fib.next[n]` 줄이다. 여기서는 `.next` 분지의 페이로드(자연수)를 수신한 뒤 `fib`를 수열의 나머지 부분으로 갱신한다. 참고로 이 코드는 *선택*과 *수신*의 두 명령을 조합한 것이다.

모두 마친 뒤에는 수열과 콘솔을 모두 닫는다. 두 값이 모두 선형이므로 이 과정은 필수이다.
