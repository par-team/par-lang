# 순서쌍

순서쌍(pair)은 두 개의 독립적인 값을 하나로 묶은 타입이다. 다른 언어와 비교해서 Par에서 특히 다른 점은 순차적 문법이라고 할 수 있다. 다소 의아할 수 있지만, 이 문법의 순서쌍이 더 다양한 용례에 적용할 수 있다는 장점이 있다.

순서쌍 타입은 두 타입으로 이루어지며, 전자는 소괄호로 감싸야 한다.

```par
module Main

import {
  @core/Int
  @core/String
}

type Pair = (String) Int
```

후자가 다른 순서쌍일 경우, 아래와 같은 *문법 설탕*을 사용해 더 간결하게 작성할 수 있다.

```par
type Triple1 = (String) (Int) String
type Triple2 = (String, Int) String
// 위의 두 타입은 완전히 같음
```

**대칭 순서쌍** 문법의 경우, [단위](./unit.md) 타입을 마지막 원소로 쓰는 것이 자연스럽다.

```par
type SymmetricPair = (String, Int)!
```

순차 스타일의 순서쌍은 다른 타입과 조합해 더 큰 자료구조에 삽입하는 데 자주 쓰인다. 내장된 `List<a>` 타입은 `.item` 선지가 순서쌍을 사용한다.

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

무한 스트림 타입은 순서쌍으로 다음 원소와 스트림의 나머지 부분을 동시에 생성할 수도 있다.

```par
type Stream<a> = iterative choice {
  .close => !,
  .next  => (a) self,
}
```

## 생성

순서쌍 값은 타입 대신 값이 들어간다는 것을 제외하면 타입과 같은 모양을 가진다.

```par
def Example1: Pair    = ("Hello!") 42
def Example2: Triple1 = ("Alice") (42) "Bob"

// `Triple1`과 `Triple2`는 정말 같은 타입임
def Example3: Triple1 = ("Alice", 42) "Bob"
def Example3: Triple2 = ("Alice", 42) "Bob"

// 마지막의 `!`에 유의
def Example4: SymmetricPair = ("Hello!", 42)!
```

순차 형태의 순서쌍은 다른 타입 사이에 자연스럽게 녹아들도록 작성할 수 있다.

```par
def Names: List<String> = .item("Alice").item("Bob").item("Cyril").end!
//                             |             |           |_____________
//                             |             |_________________________
//                             |_______________________________________
```

## 소멸

순서쌍은 패턴에 대입해서 소멸시킬 수 있다. 패턴이 올 수 있는 자리는 다음과 같다.
- [`let`식](../structure/let_expressions.md)
- [함수](./function.md) 인자
- [`case`](./choice.md)/[`.case`](./either.md) 분지

순서쌍과 전체 값 이외에도 [단위](./unit.md) 타입 역시 패턴으로 매치할 수 있다.

몇 가지 예제를 보자.

```par
def Five: Int =
  let (x) y = (3) 2
  in x + y

def FiveSymmetrically: Int =
  let (x, y)! = (3, 2)!
  in x + y

dec AddSymmetricPair : [(Int, Int)!] Int
def AddSymmetricPair = [(x, y)!] x + y
//                      \_____/<---- 이 부분이 패턴임

dec SumList : [List<Int>] Int
def SumList = [list] list.begin.case {
  .end!       => 0,
  .item(x) xs => x + xs.loop,
//     \____/<---- 이 부분이 패턴임
}
```
