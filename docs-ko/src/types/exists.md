# 존재

*존재(exists) 타입*은 **값 내부에 타입을 숨기고**, 그 타입으로 할 수 있는 것만을 드러낼 수 있다.

존재 타입은 매개변수 타입인 [*전칭*](./forall.md)의 쌍대이다. 전칭 타입 `[type a] ...`는 *아무 타입 `a`를 전달하면 그걸로 연산을 할 수 있는* 타입이었다면, *존재 타입* `(type a) ...`는 *어떤 타입 `a`가 이미 선택되어 있지만 어떤 것인지는 알 수 없는* 타입이다.

존재 타입은 아래의 **두 부분으로 구성된다.**
- 소괄호 안에 소문자 타입 변수를 적고 키워드 `type`을 앞에 붙인다.
- 바로 뒤에 숨은 타입 변수를 사용할 수 있는 페이로드 타입을 작성한다.

[순서쌍](./pair.md)과 비슷하지만, 왼쪽 원소가 값이 아니라 타입이라는 점에서 다르다. 또한 [타입 제약](./constraints.md)에서 다루었듯이 `(type a: data)`와 같이 숨은 타입에 제약을 걸 수 있다.

다음은 존재 타입을 사용하는 간단한 예제이다.

```par
type Any = (type a) a

type DropMe = (type a: drop) a
```

전자의 타입은 완전히 불투명하다. `Any` 타입의 값에는 숨은 타입을 가지는 값이 있지만, 그 값에 수행할 수 있는 연산은 딸려오지 않는다. 실제 쓸모는 없지만, 존재 타입의 가장 간단한 예제에 해당한다.

후자의 타입에서는 숨은 타입의 능력 중 하나, 즉 값을 정리할 수 있다는 것을 공개하고 있다. 이 정도로도 Par에서 숨은 값 자체가 무엇인지 모르고도 값을 정리할 수 있도록 하는 데는 충분하다.

그러면 존재 타입을 어떻게 사용하는지 확인해 보자.

## 생성

존재 값을 생성하려면 우선 구상 타입을 고르고 그 타입에 해당하는 페이로드를 준비해야 한다.

문법은 `type` 키워드가 추가된 점을 제외하면 [순서쌍](./pair.md)과 같다.

다음은 `Any` 타입의 값이다.

```par
def Hidden: Any = (type Int) 42
```

하지만 선택한 타입이 `Any`로 숨겨져 있고 주어진 다른 연산이나 제약도 없으므로, 이 값은 전혀 쓸모가 없으며 아무런 조작도 할 수 없다. 사실 이 값을 변수로 인스턴스화한다면 버리는 것조차도 불가능하다.

조금 더 흥미로운 예제를 살펴 보자.

```par
type DropMe = (type a: drop) a
```

이 타입의 페이로드는 숨은 타입을 가지는 값이다. 타입의 제약에 의해 이 값이 아무것도 하지 않아도 되는 일반 데이터인지, 정리 프로토콜을 따르는 값인지는 모르지만 안전한 구조적 정리가 가능하다는 것을 알 수 있다.

`DropMe`의 값은 다음과 같이 생성할 수 있다.

```par
def Drop42: DropMe = (type Int) 42
```

## 소멸

존재 값을 사용하려면 우선 값을 **풀어내어야 한다**. 문법은 `type` 키워드를 제외하면 순서쌍을 풀어내는 것과 같다.

다음은 `DropMe`를 풀어내서 사용하는 예제이다.

```par
def UseDrop: ! =
  let (type a: drop) x = Drop42
  in !
```

패턴 `(type a: drop) x`는 다음과 같은 의미이다.
- `a`는 숨은 타입의 이름에 해당하는 지역 타입 변수가 된다.
- `x`는 존재 값에 보관한 타입 `a`에 해당하는 값이다.

`x`를 사용하지 않으므로 Par에서 자동으로 정리한다.

**함수 매개변수**에서도 패턴을 사용해 존재 값을 풀어낼 수 있다. 다음은 `DropMe`를 전달받아 내부의 값을 버리는 함수이다.

```par
dec DropIt : [DropMe] !
def DropIt = [(type a: drop) x] !
```

## 현실적인 예제

*존재* 타입을 [박스](./box.md) 타입과 결합하면 자유롭게 전달할 수 있는 인터페이스 내부에 구현 세부사항을 숨길 수 있게 되어 쓸모 있는 타입이 된다.

다음은 집합을 조작하는 박싱된 인터페이스이다.

```par
type SetModule<a> = (type set: share) box choice {
  .empty => set,
  .insert(a, set) => set,
  .contains(a, set) => Bool,
}
```

이 타입은 집합을 구현하는 타입을 숨기고 있다. 인터페이스 자체가 박싱되어 있기 때문에 복사하거나 버릴 수 있다. 추가로 숨은 `set` 타입도 [`share`](./constraints.md)로 제약되었기 때문에 집합을 조작하는 연산 이후에도 집합 값을 재사용할 수 있다. 이때 숨은 타입 `set`은 전혀 드러나지 않으며, `set`에 대한 연산만이 노출된다.

리스트를 사용해 비효율적이지만 간단한 `SetModule`을 구현해 보자. 이 구현체는 `==`을 사용해 주어진 값이 이미 집합에 존재하는지 판정하므로 [`data`](./constraints.md) 원소를 지원한다.

```par
module Main

import {
  @core/Bool
  @core/Int
  @core/List
}

dec ListSet : [type a: data] SetModule<a>
def ListSet = [type a: data] (type List<a>) box case {
  .empty => .end!,

  .insert(x, set) => .item(x) set,

  .contains(y, set) => set.begin.case {
    .end! => .false!,
    .item(x) xs => {x == y}.case {
      .true! => .true!,
      .false! => xs.loop,
    },
  },
}
```

위의 집합 구현체는 비교가 가능한 데이터 타입에 대해서만 생성할 수 있으며, `a: data`에 의해 이 성질이 보장된다.

존재 값을 생성하면서 `List<a>`를 숨은 타입으로 선택하고 있다.

```par
(type List<a>) ...
```

`SetModule`의 사용자는 집합이 리스트로 구현되어 있음을 알 수 없다.

이제 이 모듈을 함수에서 사용해 보자.

```par
dec Deduplicate : [type a: data, SetModule<a>, List<a>] List<a>
def Deduplicate = [type a: data, (type set: share) mSet, list]
  let visited = mSet.empty
  in list.begin.case {
    .end! => .end!,
    .item(x) xs => mSet.contains(x, visited).case {
      .true! => xs.loop,
      .false! =>
        let visited = mSet.insert(x, visited)
        in .item(x) xs.loop,
    }
  }
```

이 함수는 숨은 집합 구현체를 사용해 거쳤던 값을 추적함으로써 리스트의 중복을 제거한다. 함수에서는 집합이 `.empty`, `.insert`, `.contains`를 지원한다는 것을 제외하면 집합이 어떻게 동작하는지 알지 못한다.

직접 시험해 보자!

```par
def IntListSet = ListSet(type Int)

def TestDedup =
  Deduplicate(
    type Int,
    IntListSet,
    List.Map(Int.Range(1, 1000), box [n] Int.Mod(n, 7)),
  )
```

위의 코드에서는 정수의 리스트에서 모듈로 7을 기준으로 중복을 제거하고 있다.
- `List.Map`으로 나머지를 구하고,
- `ListSet`을 통해 추상 집합 구현체를 사용하며,
- `Deduplicate`으로 중복을 제거한다.

집합의 표현을 노출하지 않으면서도 모든 연산을 수행할 수 있었다.
