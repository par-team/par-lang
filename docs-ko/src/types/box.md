# 박스

Par는 **선형 타입 시스템**을 가지고 있어, 원칙적으로는 값을 **정확히 한 번** 사용해야 한다.

하지만 모든 값에 원칙만을 고수할 필요는 없다. 프로그램을 작성하다 보면 다음과 같은 동작이 필요하다.
- 여러 번 호출할 수 있는 함수를 전달하기
- 필요 없는 값을 버리기
- 고차 유틸리티를 자유롭게 합성하기

바로 여기에 **박스(box) 타입**이 쓰인다.

## 박스 없이도 공유 가능한 타입

Par의 몇몇 타입은 박스 타입 없이도 **공유가 가능**하다. 여기에는 데이터 타입이 속한다.
- [단위](./unit.md)
- [분기](./either.md)
- [순서쌍](./pair.md)
- [재귀](./recursive.md)
- 모든 [원시 타입](../structure/primitive_types.md): `Int`, `Nat`, `Float`, `String`, `Char`, `Byte`, `Bytes`

위의 타입은 공유 타입으로만 조합하는 한 어떻게 조합하든 공유 타입이므로 자유롭게 복사하고 버릴 수 있다.

그러나 [**함수**](./function.md), [**선택**](./choice.md) 등의 비데이터 타입이 포함된 타입은 얼마나 깊숙이 포함되어 있든 선형이 된다. 다만 사용성을 향상하기 위해 일부 선형 선택 값은 [자동 정리](./auto_cleanup.md)로써 버릴 수 있도록 하였다. 나중에 해당 장에서 자세히 설명할 예정이다.

이제 이런 상황을 생각해 보자. **리스트의 원소마다 같은 함수를 적용**하고 싶다면 어떻게 해야 할까?

Par의 일반적인 함수는 선형으로 한 번만 사용할 수 있기 때문에, 같은 함수를 여러 번 적용하려면 다른 방법이 필요하다.

## 불편하게 구현한 다회용 함수

박스가 없다면 함수의 사용 프로토콜을 직접 인코딩해서 다회용 함수를 만들 수 있다.

```par
type Mapper<a, b> = iterative choice {
  .close => !,
  .apply(a) => (b) self,
}
```

이 프로토콜은 다음 두 메서드를 제공한다.
- 함수를 사용하는 `.apply`
- 함수를 정리하는 `.close`

다음은 이 프로토콜을 사용하는 `Map` 함수와...

```par
dec Map : [type a, type b, List<a>, Mapper<a, b>] List<b>
def Map = [type a, type b, list, mapper] list.begin.case {
  .end! => let ! = mapper.close in .end!,
  .item(x) xs => let (x1) mapper = mapper.apply(x) in .item(x1) xs.loop,
}
```

함수를 실제로 사용하는 모습이다.

```par
def NumberStrings = Map(type Int, type String, Int.Range(1, 100), begin case {
  .close => !,
  .apply(n) => (`#{n}`) loop,
})
```

목적은 달성했지만, 지나치게 번잡하다.

다회용 함수를 사용할 때마다 `Mapper`와 같은 프로토콜에 수동으로 인코딩해야 한다. 복제하고, 닫고, 연쇄하는 것도 모두 수동 작업이 되었다.

## 편리한 박스 타입

Par에서는 다회용 값을 수동으로 인코딩하는 대신 값을 박스 처리할 수 있다.

**임의의** 타입 `T`에 대해 `box T`는 공유 타입으로 취급되는 `T` 타입에 해당한다. 이 타입은,

- **복사하거나**,
- **버리거나**,
- 다른 곳에 전달하는 것이 모두 자유롭다.

박싱된 값은 다음과 같이 생성할 수 있다.

```par
box <expression>
```

본문 식의 타입 `T`에 대해 `box T` 타입의 값이 생성된다.

**`box`식 안에서는 공유 변수만 포착할 수 있다**는 한 가지 규칙만 지키면 된다.

여기에는 다음과 같은 값이 포함된다.
- 데이터 타입 (`Int`, `String`, `List<Int>` 등)
- 다른 `box` 값

이때 ***포착***이란 *식 밖에서 생성된 지역 변수*를 *식 안에서 사용*하는 것을 말한다.

## 다시 구현한 `Map`

박스를 사용하면 위에서 보았던 `Map` 함수 역시 더 깔끔하게 구현할 수 있다.

```par
module Main

import @core/List

dec Map : [<a> List<a>, <b> box [a] b] List<b>
def Map = [<a> list, <b> f] list.begin.case {
  .end! => .end!,
  .item(x) xs => .item(f(x)) xs.loop,
}
```

직접 사용해 보자.

```par
def NumberStrings = Map(Int.Range(1, 100), box [n] `#{n}`)
```

래퍼도, 수동 프로토콜도 필요 없었다. `box`는 어떤 타입이든 공유 타입으로 만드므로 박싱된 함수를 자유롭게 사용할 수 있다. 이것이 바로 `box`의 주 용도이다.

## 서브타입 관계

박스 타입은 Par의 서브타입 체계에 자연스럽게 녹아든다.

**`box T`는 `T`가 올 자리라면 어디든지 사용할 수 있다.**

```par
def BoxInt: box Int = 42       // 올바른 코드 (Int는 공유 가능)
def UseInt: Int = BoxInt       // 올바른 코드 (box Int는 Int처럼 사용할 수 있음)
```

또한, **`T`가 원래 공유 타입이라면 `T` 역시 `box T`가 올 자리 어디든지 사용할 수 있다.**

```par
def Boxes: List<box Int> = *(1, 2, 3)
def Ints: List<Int> = Boxes
```

**원래부터 공유 타입인 경우, `T`와 `box T`는 사실상 아무런 차이가 없다.**

## 다른 예제: 리스트 필터링

이번에는 박싱된 조건 함수에 따라 리스트를 필터링하는 함수를 작성해 보자.

이 예제에서는 `share` 타입 제약을 사용하며, `a: share`로 작성되어 있다. 다음 장에서 타입 제약을 자세히 다루며, 지금은 '원소 타입이 공유 타입이어야 한다' 정도로만 이해하면 충분하다.

```par
module Main

import {
  @core/Bool
  @core/Int
  @core/List
}

dec Filter : [<a: share> List<a>, box [a] Bool] List<a>

def Filter = [<a: share> list, predicate] list.begin.case {
  .end! => .end!,
  .item(x) xs => predicate(x).case {
    .true! => .item(x) xs.loop,
    .false! => xs.loop,
  }
}
```

타입에 주목해 보면,
- `List<a>`를 인자로 받는다.
- `a: share` 제약에 의해 원소를 복사하거나 버릴 수 있다.
- 반환값 역시 `List<a>`이다.

직접 사용해 보자.

```par
def Evens = Filter(Int.Range(1, 100), box [n] {Int.Mod(n, 2) == 0})
```

이때,
- `Int.Range(1, 100)`의 결과는 `List<Int>`이다.
- 정수는 비선형이므로 `Int`는 `share` 제약을 만족한다.
- 계산 결과는 `List<Int>`로 추론된다.

`Filter`를 구현할 때 왜 `drop`이 아니라 더 강한 `share`가 필요할까? `x`는 `predicate(x)`에서 한 번 사용하고, `.true!`를 반환했을 때 결과로 반환할 목적으로 하나를 더 유지하기 때문이다. 즉, 이 구현에서는 값을 버리는 것뿐만 아니라 복사하는 능력도 필요하다.

`a: share` 제약에 의해 모든 원소를 불필요하게 `box`로 감싸지 않아도 구현체의 요구사항이 충족되므로 리스트의 타입을 깔끔하게 유지할 수 있다.
