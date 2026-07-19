# 단위

단위(unit) 타입은 `!`라고 작성하며, 여기에 속하는 하나의 값 역시 `!`라고 작성한다.

```par
def Unit: ! = !
```

단위 타입은 다른 타입의 종결자로 자주 쓰인다. [순서쌍](./pair.md)이나 [분기](./either.md), [선택](./choice.md)과 같은 모든 합성 타입은 *'그 다음'* 부분을 필수적으로 요구하는데, 단위 타입으로써 <em>'그걸로 끝'</em>의 경우를 나타낼 수 있다.

예를 들어, 내장된 `List<a>` 타입의 정의는 다음과 같다.

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

[분기](./either.md) 타입의 모든 선지는 페이로드를 필수적으로 작성해야 하는데, 리스트의 끝을 나타내는 노드의 경우 페이로드가 없으므로 `!`로 표시한다.

## 생성

식 `!`가 타입 `!`를 가지며, 그 이외에 이 타입을 가지는 값은 없다.

```par
def Unit = !  // `Unit`의 타입이 `!`으로 추론됨
```

## 소멸

단위 타입은 [비선형](../types_and_expressions.md#선형성)이므로 타입 `!`의 변수는 사용하지 않고 버려도 된다.

복잡한 타입에 속하는 `!`를 다룰 경우에는 패턴에 대입해서 사용해야 할 수도 있다. 이때는 패턴 `!`를 사용해 `!` 값을 특정한 변수에 대입하지 않고 소멸시킬 수 있다.

```par
def TestUnitDestruction = do {
  let unit = !
  let ! = unit
} in !
```

이는 특히 리스트의 끝에 매치할 때나...

```par
module Main

import {
  @core/Int
  @core/List
}

dec GetFirstOrZero : [List<Int>] Int
def GetFirstOrZero = [list] list.case {
  .end!      => 0,  // 여기서 사용한 `!`는 패턴이다
  .item(x) _ => x,
}
```

`!`로 끝나는 [순서쌍](./pair.md)을 소멸시킬 때 유용하다.

```par
module Main

import @core/Int

dec SumPair : [(Int, Int)!] Int
def SumPair = [pair]
  let (x, y)! = pair  // 여기서 사용한 `!`는 패턴이다
  in x + y

def Five =
  let pair = (2, 3)!
  in SumPair(pair)
```
