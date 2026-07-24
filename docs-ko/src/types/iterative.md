# 반복

두 가지 자기 참조 타입 중 [*재귀 타입*](./recursive.md)은 이미 배운 바가 있다. 이번에는 나머지 하나인 *반복(iterative) 타입*을 살펴본다. 이 타입은 [*쌍대귀납*](https://en.wikipedia.org/wiki/Coinduction), 혹은 [*쌍대재귀*](https://en.wikipedia.org/wiki/Corecursion)를 구현할 수 있다는 점에서 쌍대재귀 타입이라고도 한다.

핵심을 요약하자면,
- **재귀 타입**의 값은 *정해진* 횟수만큼 반복할 수*만 있다*.
- **반복 타입**의 값은 *원하는* 횟수만큼 반복할 수 *있다*.

재귀 타입의 값은 몇 번 반복하면 끝까지 갈 수 있는지 알 수 있다. 이 반복을 풀어내는 것이 `.begin`/`.loop`의 역할이다. `self` 값이 있다면 항상 `.loop`를 통해 끝까지 재귀할 수 있다.

하지만 반복 타입의 값은 몇 번 반복할지를 *알려주어야* 한다. 반복 값은 아무리 많은 횟수라도 반복할 수 있는 능력이 있고, 어디까지 갈지를 결정하는 것은 사용자인 우리의 몫이다.

이제 **반복 타입**을 직접 확인해 보자.

반복 타입은 키워드 `iterative` 다음에 `self`를 원하는 횟수만큼 포함하는 본문을 작성하면 된다. [재귀 타입](./recursive.md)과 같은 구조를 공유한다.

반복 타입의 대표적인 예시이자 좋은 학습 자료로 무한 수열을 들 수 있다.

```par
type Sequence<a> = iterative choice {
    .close => !,
    .next => (a) self,
}
```

> **반복(이나 [재귀](./recursive.md)) 타입이 중첩되어 있다면, 여러 개 중 하나를 구분해야 할 수 있다.** 이때는 `iterative`와 `self`에 **레이블**을 추가할 수 있으며, 골뱅이표를 사용한다(`iterative@label`, `self@label`). 어떤 소문자 식별자든 레이블로 사용할 수 있다.

반복 타입은 본문의 내용과 무관하게 **항상 선형**이므로, 버리거나 복사할 수 없다.

위에서 정의한 `Sequence`를 보면 내부의 [선택](./choice.md) 타입에 `.close` 분지를 포함한 것을 확인할 수 있다. `Sequence<a>`는 선형 타입이므로 `.next` 분지만 있었다면 값을 완전히 소멸시키는 것이 불가능했을 것이다.

반복 타입 역시 [재귀 타입](./recursive.md)과 마찬가지로 `self`를 *전개*시킨 형태와 동치시킬 수 있다.

1. 원래 정의:
   ```par
   type Sequence<a> = iterative choice {
     .close => !,
     .next => (a) self,
   }
   ```
2. 1차 전개:
   ```par
   type Sequence<a> = choice {
     .close => !,
     .next => (a) iterative choice {
       .close => !,
       .next => (a) self,
     },
   }
   ```
3. 2차 전개:
   ```par
   type Sequence<a> = choice {
     .close => !,
     .next => (a) choice {
       .close => !,
       .next => (a) iterative choice {
         .close => !,
         .next => (a) self,
       },
     },
   }
   ```
4. 등등...

> [재귀](./recursive.md) 타입과 같이, 반복 타입에도 중요한 제한이 있다. **반복 타입의 모든 `self` 참조는 `choice`로 감싸져 있어야 한다.** `choice`가 `iterative` 바로 다음에 오지 않아도 되지만, `iterative`와 `self` 사이 *어딘가*에는 삽입되어 있어야 한다.
>
> ```par
> type ValidSequence<a> = iterative (a) choice {
>   .close => !,
>   .next => self,  // 올바른 코드. `self`가 `choice`로 감싸져 있다.
> }
> 
> type InvalidSequence<a> = iterative (a) self  // 오류! `self` 참조가 빠져나옴
> ```

[재귀](./recursive.md) 타입과 반복 타입이 모두 전개가 가능한데, 그러면 **둘의 차이는 무엇일까?** 두 타입의 차이는 생성과 소멸 방식에 있다.
- **재귀 타입**은 **수동으로 생성**하고 **루프로 소멸**시키지만,
- **반복 타입**은 **루프로 생성**하고 **수동으로 소멸**시킨다.

무슨 의미인지 자세히 살펴 보자!

## 생성

반복 타입의 값은 `begin`/`loop`문을 단독으로 작성해 생성한다. `begin` 다음에 본문 타입의 식을 작성하면 된다. 식 안에서 대응하는 `begin`으로 돌아가려면 본문 타입의 `self` 자리에 `loop`를 단독으로 작성하면 된다.

> [재귀](./recursive.md) 타입의 `.begin`/`.loop`와 같이 중첩된 `begin`/`loop`문(과 `.begin`/`.loop`)도 레이블로 구분할 수 있다. `begin@label`과 `loop@label`과 같이 골뱅이표를 사용하면 된다.

다음은 정수 `7`을 무한히 생성하는 간단한 `Sequence<Int>` 값이다.

```par
module Main

import @core/Int

dec SevenForever : Sequence<Int>
def SevenForever = begin case {
  .close => !,
  .next  => (7) loop,
}
```

`.next` 분지는 수열의 본문 타입을 따라 순서쌍을 만들며, 그 오른쪽 원소는 새로운 버전의 수열이다. 여기서 `loop`를 사용해 `begin`으로 돌아가 쌍대재귀적으로 수열을 계속 생성해 낸다.

`begin`/`loop`의 쌍대재귀적 의미 역시 식을 전개해서 이해할 수 있다.

1. 원래 코드:
   ```par
   def SevenForever = begin case {
     .close => !,
     .next  => (7) loop,
   }
   ```
2. 1차 전개:
   ```par
   def SevenForever = case {
     .close => !,
     .next  => (7) begin case {
       .close => !,
       .next  => (7) loop,
     },
   }
   ```
3. 2차 전개:
   ```par
   def SevenForever = case {
     .close => !,
     .next  => (7) case {
       .close => !,
       .next  => (7) begin case {
         .close => !,
         .next  => (7) loop,
       },
     },
   }
   ```
4. 등등...

**지역 변수의 유지** 역시 [재귀](./recursive.md) 값의 `.begin`/`.loop`와 동일하다. 반복 타입에서는 이 기능을 사용해 반복 값의 **내부 상태를 갱신**할 수 있다.

피보나치 무한 수열을 예시로 들어 보자.

```par
module Main

import @core/Nat

def Fibonacci: Sequence<Nat> =
  let (a, b)! = (0, 1)!
  in begin case {
    .close => !,
    .next =>
      let (a, b)! = (b, a + b)!
      in (a) loop
  }
```

이 기능은 매우 유용하다. 대다수의 프로그래밍 언어에서 이와 비슷한 `Fibonacci` 객체를 구현하려면 내부 상태를 기술하는 `class`나 `struct`를 정의한 뒤 메서드 내에서 갱신하여야 한다. Par에서는 내부 상태에 별도의 타입을 사용할 필요 없이 익명의 식으로 반복 객체를 생성할 수 있다. **내부 상태는 그저 지역 변수에 불과하다.**

`Fibonacci`의 경우에는 내부 상태가 비선형이다. `.close` 분지가 선택되면 `a`와 `b`가 자동으로 버려지기 때문에 별도의 처리 없이 단위 값으로 바로 소멸시키는 것이 가능하다.

내부 상태가 선형인 경우도 살펴 보자! 임의의 정수열을 입력받은 뒤, 모든 원소에 `1`을 더해서 새로운 수열을 생성하는 함수를 구현해 보자.

```par
dec Increment : [Sequence<Int>] Sequence<Int>
def Increment = [seq] begin case {
  .close => let ! = seq.close in !,
  .next =>
    let (x) seq = seq.next
    in let x = x + 1
    in (x) loop
}

def FibonacciPlusOne = Increment(Fibonacci)
```

이 경우에는 `.close` 분지에서 입력받은 `seq` 값을 명시적으로 닫아야 한다. 이 값은 선형이므로 마음대로 버릴 수 없다.

### 전체성의 임시방편, `.unfounded`

[재귀 소멸](./recursive.md#전체성의-임시방편-unfounded)과 마찬가지로 반복 생성의 경우에도 전체성을 띰(아무런 진행을 하지 않는 무한루프에 진입하지 않음)을 알 수 있지만 Par의 타입 검사기가 거부하는 경우가 있을 수 있다. 이 경우에는 `begin`을 `unfounded`로 바꾸면 전체성 확인을 비활성화할 수 있다.

## 소멸

반복 타입은 소멸 문법이 따로 없으며, 반복 타입이 전개된 것처럼 취급해서 내용물을 직접 조작하면 된다.

다음은 주어진 수열에서 첫 원소를 취한 뒤 바로 닫는 함수이다.

```par
def Head = [type a, seq: Sequence<a>]
  let (x) seq = seq.next
  in let ! = seq.close
  in x
```

[재귀](./recursive.md)를 활용하면 반복 타입을 여러 겹 소멸시킬 수 있다. 다음은 주어진 수열의 처음 N개의 원소를 리스트로 옮겨서 반환하는 함수이다.

```par
dec Take : [type a, Nat, Sequence<a>] List<a>
def Take = [type a, n, seq] Nat.Repeat(n).begin.case {
  .end! => let ! = seq.close in .end!,
  .step remaining =>
    let (x) seq = seq.next
    in .item(x) remaining.loop
}
```
