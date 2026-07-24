# 타입 제약

제네릭 코드를 작성할 때는 모르는 타입에 대해 약간의 정보 정도는 필요한 경우가 많다.

예를 들어, 이 함수는 `a`에 대해 아무것도 모르더라도 인자를 그대로 반환하는 데 문제가 없지만...

```par
dec Identity : [<a> a] a
def Identity = [<a> x] x
```

이 함수는 인자를 두 번 사용해야 한다.

```par
dec Duplicate : [<a: box> a] (a, a)!
def Duplicate = [<a: box> x] (x, x)!
```

위 함수의 `: box` 부분이 **타입 제약**이다. 여기서는 알 수 없는 타입 `a`의 값을 재사용하거나 버릴 수 있도록 비선형이라는 제약을 추가하고 있다.

Par에서는 현재 다음 네 가지의 제약을 지원한다.

- `box` (박스)
- `data` (데이터)
- `number` (수치)
- `signed` (부호)

위의 제약은 강도 순서대로 위계를 이룬다.

```text
signed -> number -> data -> box
```

즉, 모든 `signed` 타입은 `number`를, `number`는 `data`를, `data`는 `box`를 항상 만족한다.

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
type SomeData = (type a: data) a
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
type Boxed<a> = box a      // 올바른 코드
type Bad<a: box> = box a   // 오류
```

타입 정의에 제약된 동작이 필요하다면, 그 타입의 값을 조작하는 함수에 제약을 추가하면 된다.

## `box` (박스) 제약

`box`는 비선형인 값에 해당하는 제약이다. 타입에 대한 정보가 `a: box`뿐인 값은 복사·재사용하거나 버리는 것이 가능하다.

```par
dec KeepFirst : [<a: box> (a, a)!] a
def KeepFirst = [<a: box> (first, second)!] first
```

위의 예제에서는 `second`를 사용하지 않았으며, `a: box`에 의해 허용되는 동작이다.

`box`를 만족하는 타입은 다음과 같다.

- 원시 타입
- `!`
- `box`를 만족하는 타입으로 이루어진 순서쌍, 분기, 재귀 타입
- 명시적 `box T`
- `box` 타입으로 전개되는 타입 동의어
- `box`, `data`, `number`, `signed` 제약 중 하나를 만족하는 타입 변수

함수, 선택, 후속문, 무제약 타입 변수는 명시적으로 `box`로 감싸지 않으면 `box`를 만족하지 않는다.

## `data` (데이터) 제약

`data`는 일반적인 데이터 값에 해당하는 제약이다. 데이터 값은 비선형이며, 추가로 다음 연산을 지원한다.

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

- 모든 원시 타입
- `!`
- 데이터로 이루어진 순서쌍
- 데이터 페이로드를 가지는 분기
- 데이터 본문을 가지는 재귀 타입
- 데이터로 전개되는 타입 동의어
- `data`, `number`, `signed` 제약 중 하나를 만족하는 타입 변수

비데이터 타입은 `box`로 감싸도 데이터가 되지 않는다.

```par
box [Int] Int  // box이지만 data는 아님
```

`T`가 이미 데이터일 경우, 박싱된 데이터도 내부의 데이터 값을 사용할 수 있으므로 `box T`는 데이터로 사용할 수 있다.

## `number` (수치) 제약

수치를 다루는 제네릭 코드에서는 `number` 제약을 사용할 수 있다. `number` 타입은 다음 연산을 지원한다.

- `+`
- `*`
- `/`
- `Number.Zero(type a)`

```par
module Main

import @core/Number

dec SumPair : [<a: number> (a) a] a
def SumPair = [<a: number> (left) right] left + right

dec Zero : [type a: number] a
def Zero = [type a: number] Number.Zero(type a)
```

수치 타입은 다음과 같다.

- `Nat`
- `Int`
- `Float`

위의 타입 중 하나로 전개되는 타입 동의어 혹은 `number`나 `signed` 제약을 만족하는 타입 변수도 `number`를 만족한다.

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

위의 타입 중 하나로 전개되는 타입 동의어 혹은 `signed` 제약을 만족하는 타입 변수도 `signed`를 만족한다.

자연수인 `Nat`은 부호 타입에서 제외된다.

## 제약의 선택

함수의 요구사항을 만족하는 가장 약한 제약을 선택하면 된다.

- 값을 복사·재사용하거나 버리기만 할 경우 `box`를 사용한다.
- 값끼리 비교하거나 `#{...}` 보간을 사용할 경우 `data`를 사용한다.
- 제네릭 영(0)을 사용하거나 덧셈, 곱셈, 나눗셈을 할 경우 `number`를 사용한다.
- 추가로 뺄셈이나 부호 반전을 할 경우 `signed`를 사용한다.

이 규칙을 따르면 최대한 유연한 API를 구현하면서 구현체의 기능 역시 드러낼 수 있다.
