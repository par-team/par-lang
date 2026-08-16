# 타입 제약

제네릭 코드를 작성할 때는 모르는 타입에 대해 약간의 정보 정도는 필요한 경우가 많다.

예를 들어, 이 함수는 `a`에 대해 아무것도 모르더라도 인자를 그대로 반환하는 데 문제가 없지만...

```par
dec Identity : [<a> a] a
def Identity = [<a> x] x
```

이 함수는 인자 중 하나를 사용하지 않을 능력이 필요하다.

```par
dec KeepFirst : [<a: drop> (a, a)!] a
def KeepFirst = [<a: drop> (first, second)!] first
```

위 함수의 `: drop` 부분이 **타입 제약**이다. 여기서는 알 수 없는 타입 `a`를 버릴 수 있는 안전한 방법이 있다는 제약을 추가해 `second`를 자동으로 정리할 수 있도록 한다.

Par에서는 다음 다섯 가지의 제약을 지원한다.

- `drop` (정리)
- `share` (공유)
- `data` (데이터)
- `number` (수치)
- `signed` (부호)

위의 제약은 강도 순서대로 위계를 이룬다.

```text
signed -> number -> data -> share -> drop
```

모든 `signed` 타입은 `number`를, `number`는 `data`를, `data`는 `share`를, `share`는 `drop`을 항상 만족한다.

위의 위계에서 오른쪽으로 갈수록 제네릭 코드가 알 수 있는 정보가 적어진다. `drop` 값은 버릴 수는 있지만, 복사, 비교, 출력, 가산 따위가 가능하다는 보장은 없다.

## 문법

제약은 타입 매개변수 다음에 쌍점으로 구분해서 적는다.

명시적 제네릭 함수에서는 `type` 대입자에 적는다.

```par
dec ZeroOr : [type a: number, Bool, a] a
```

암시적 제네릭 함수에서는 부등호 대입자에 적는다.

```par
dec Sum : [<a: number> (a) a] a
```

존재 타입에서도 숨은 타입에 제약을 걸 수 있다.

```par
type SomeDroppable = (type a: drop) a
```

암시적 제네릭 순서쌍도 동일하다.

```par
type DataWithText = (<a: data> a) String
```

명시적 대입자에 제약이 있는 값을 생성할 때는 검사되는 타입에도 같은 제약을 추가해야 한다.

```par
dec ShowTwice : [type a: data, a] String
def ShowTwice = [type a: data, x] `#{x} #{x}`
```

타입 매개변수를 제약할 수 없는 곳으로는 기명 타입 정의가 있다.

```par
type Boxed<a> = box a        // 올바른 코드
type Bad<a: share> = box a   // 오류
```

타입 정의에 제약된 동작이 필요하다면, 그 타입의 값을 조작하는 함수에 제약을 추가하면 된다.

## `drop` (정리) 제약

`drop`은 사용하지 않고 버릴 수 있는 값에 해당하는 제약이다. 공유 값에 대해서는 별도의 정리가 필요하지 않지만, 선형 값은 Par에서 [구조적 정리](./auto_cleanup.md)를 수행한다.

```par
dec KeepFirst : [<a: drop> (a, a)!] a
def KeepFirst = [<a: drop> (first, second)!] first
```

`KeepFirst`에서는 두 값 중 어느 것도 복사할 필요가 없으므로 이 함수가 필요한 능력은 `drop`으로 충분하다. 이제 이 함수는 일반적인 데이터뿐만 아니라 정리가 가능한 자원에도 호출할 수 있다.

표준 라이브러리의 `List.Length`에도 같은 형태의 요구사항이 있다.

```par
dec List.Length : [<a: drop> List<a>] Nat
```

이 함수는 리스트를 순회하면서 내용물을 남기지 않고 노드의 개수를 센다. 순수 선형 값이었을 경우에는 이 동작이 불가능하고, `drop` 제약을 추가하면 충분하다.

`drop`을 만족하는 타입은 다음과 같다.

- 원시 타입, `!`, 모든 `share` 타입
- 명시적 `box T`
- `drop`을 만족하는 타입으로 이루어진 순서쌍, 분기 타입
- `self`가 `drop`임을 가정했을 때 본문이 `drop`을 만족하는 재귀 타입
- `self`가 `drop`임을 가정하지 않아도 본문이 `drop`을 만족하는 반복 타입
- 정리 분기의 결과가 `drop`을 만족하는 선택 타입
- 본문이 `drop`을 만족하는 제네릭 타입
- `drop` 이상의 제약이 적용된 타입 변수

함수나 후속문, 가용한 정리 분지가 없는 선택, 무제약 타입 번수는 `drop`을 만족하지 않는다.

## `share` (공유) 제약

`share`는 복사·재사용하거나 버릴 수 있는 값에 해당하는 제약이다.

```par
dec Duplicate : [<a: share> a] (a, a)!
def Duplicate = [<a: share> x] (x, x)!
```

여기서는 `drop`만으로는 모자라다. 반환하는 순서쌍을 생성할 때 `x`를 두 번 사용하므로 `Duplicate`를 구현할 때는 `share`가 필요하다.

값을 서로 다른 경로에서 여러 번 사용할 때는 구별이 쉽지 않을 수 있다. `List.Filter`를 보자.

```par
dec Filter : [<a: share> List<a>, box [a] Bool] List<a>
def Filter = [<a: share> list, predicate] list.begin.case {
  .end! => .end!,
  .item(x) xs => predicate(x).case {
    .true! => .item(x) xs.loop,
    .false! => xs.loop,
  }
}
```

`prediacte`에서 `x`를 한 번 사용하고, 여기서 `.true!`를 반환할 경우 출력 리스트에서 `x`를 한 번 더 사용한다. 이는 복사에 해당하므로 `.false!` 경로에서 값을 버리기만 하더라도 `share` 이상의 제약이 필요하다.

`share`를 만족하는 타입은 다음과 같다.

- 원시 타입과 `!`
- `share`를 만족하는 타입으로 이루어진 순서쌍, 분기, 재귀, 반복 타입
- `T`에 관계없이 모든 `box T`
- 본문이 `share`를 만족하는 명시적·암시적 제네릭 타입
- `box`, `data`, `number`, `signed` 제약 중 하나를 만족하는 타입 변수

함수, 선택, 후속문, 무제약 타입 변수는 명시적으로 `box`로 감싸지 않으면 `share`를 만족하지 않는다.

## `data` (데이터) 제약

`data`는 비교나 출력이 가능한 일반적인 데이터 값에 해당하는 제약이다. 데이터 값은 공유 가능하며, 추가로 다음 연산을 지원한다.

- 비교 연산자: `<`, `>`, `<=`, `>=`, `==`, `!=`
- 템플릿 문자열에서 데이터 보간: `#{...}`

```par
dec Min : [<a: data> (a) a] a
def Min = [<a: data> (left) right] if {
  left <= right => left,
  else => right,
}

dec Label : [<a: data> a] String
def Label = [<a: data> value] `value = #{value}`
```

비교 연산자는 배후에서 `@core/Data.Compare`를 사용한다.`#{...}` 템플릿 보간은 `@core/Data.ToString`을 사용한다.

`data`를 만족하는 타입은 다음과 같다.

- 모든 원시 타입과 `!`
- 데이터로 이루어진 순서쌍
- 데이터 페이로드를 가지는 분기
- 데이터 본문을 가지는 재귀 타입
- `data`, `number`, `signed` 제약 중 하나를 만족하는 타입 변수

비데이터 타입은 `box`로 감싸도 데이터가 되지 않는다.

```par
box [Int] Int  // share이지만 data는 아님
```

`T`가 이미 데이터일 경우, 박싱된 데이터도 내부의 데이터 값을 사용할 수 있으므로 `box T`는 데이터에 해당한다.

## `number` (수치) 제약

수치를 다루는 제네릭 코드에서는 `number` 제약을 사용할 수 있다. `number` 타입은 다음 연산을 지원한다.

- `+`
- `*`
- `/`
- `Number.Zero(type a)`

```par
module Main

import {
  @core/List
  @core/Number
}

dec Sum : [<a: number> List<a>] a
def Sum = [<a: number> list] list.begin.case {
  .end! => Number.Zero(type a),
  .item(x) xs => x + xs.loop,
}
```

수치 타입은 다음과 같다.

- `Nat`
- `Int`
- `Float`

`Nat`은 수치 타입이지만 부호가 없으므로 `number`에서는 뺄셈이나 부호 반전을 지원하지 않는다.

## `signed` (부호) 제약

음수 값을 지원하는 타입은 `signed` 제약을 추가로 만족하며, `number`의 모든 연산뿐만 아니라 다음을 추가로 지원한다.

- `-`
- `neg`

```par
dec Difference : [<a: signed> (a) a] a
def Difference = [<a: signed> (left) right] left - right

dec Negate : [<a: signed> a] a
def Negate = [<a: signed> value] neg value
```

부호 타입은 다음과 같다.

- `Int`
- `Float`

자연수인 `Nat`은 부호 타입에서 제외된다.

## 제약의 선택

제네릭 인자를 전달받을 때는 함수의 요구사항을 만족하는 가장 약한 제약을 선택하면 된다.

- 값을 버리기만 할 경우 `drop`을 사용한다.
- 값을 복사하거나 재사용할 경우 `share`를 사용한다.
- 값끼리 비교하거나 `#{...}` 보간을 사용할 경우 `data`를 사용한다.
- 제네릭 영(0)을 사용하거나 덧셈, 곱셈, 나눗셈을 할 경우 `number`를 사용한다.
- 추가로 뺄셈이나 부호 반전을 할 경우 `signed`를 사용한다.

거꾸로 존재 값을 생성할 때는 가장 강한 제약을 선택하면 된다.
