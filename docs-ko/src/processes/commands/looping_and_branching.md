# 반복과 분기

지금까지는 [선택 타입](../../types/choice.md)과 [함수 타입](../../types/function.md)에 명령(선택과 송신)을 전달하는 법을 배웠다. 하지만 명령을 내릴 수 있는 타입은 이뿐만이 아니다. 모든 타입에 대응하는 명령이 있다.

이번에는 조금 더 어려운 타입인 **재귀 타입**으로 눈을 돌려 보자.

다음은 지금까지 자주 다루었던 리스트 타입이다.

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

`List` 타입은 [재귀](../../types/recursive.md) 타입 내에 본문으로 [분기](../../types/either.md)가 있으며, 그 안에는 [순서쌍](../../types/pair.md)을 포함하고 있다.

프로세스 문법에서 이런 자료구조를 다루려면 다음 세 종류의 명령을 조합해야 한다.

- `.begin`과 `.loop`를 사용한 재귀
- 분기 타입에 대해 `.case` 분기
- 순서쌍 타입에 대해 `value[variable]`로 값 추출

지금부터 문자열 연결 함수를 구현하면서 위의 모든 명령을 시연하도록 하겠다.

```par
module Main

import {
  @core/List
  @core/String
}

dec Concat : [List<String>] String
```

이 함수는 문자열의 리스트를 전달받고, 리스트의 모든 문자열을 연결해서 반환한다. 프로세스 문법을 사용하면서 코드가 어떻게 작성되는지 확인해 보자!

```par
def Concat = [strings] do {
  // 코드 작성 시작!
```

**차근차근 확인해 보자!**

## 프로세스 문법에서의 재귀

우선은 프로세스에 *지금부터 재귀 값을 다룰 것*이라는 의도를 전달해야 한다.

`.begin` 명령이 바로 이 역할을 한다. `.begin`은 루프의 원점으로서 나중에 `.loop`을 사용했을 때 돌아올 수 있는 역할을 한다.

```par
  strings.begin
```

## 분기

`.case` 명령은 식 문법에서의 `.case`와 비슷하지만, 두 가지 차이점이 있다. 프로세스 문법에서는 각 `.case` 분지의 **본문**이 **값이 아니라 프로세스**이다.

즉, 분지의 `=>` 다음에는 중괄호 블록이 와야 하고, 그 안에서는 값을 반환하는 것이 아니라 어떤 동작을 한다.

```par
  strings.case {
    .end! => {
      // 빈 리스트, 할 것이 없음
    }
    .item => {
```

또 다른 점을 찾을 수 있다. `.item` 분지를 보면 분지 이름 바로 뒤에 패턴 없이 `=>`가 따라붙는다! 프로세스 문법에서는 [분기](../../types/either.md)의 페이로드를 대입시키는 것이 필수가 아니기 때문이다. 이때는 **주어 자체가 페이로드가 된다**. 단, 원한다면 페이로드 전체를 매치하는 것도 가능하다.

그러므로 `.item` 분지 안에서는 `strings`가 `(String) List<String>` 타입을 가지게 된다. 이 타입은 순서쌍이고, 왼쪽 원소를 추출해 문자열 빌더에 전달해야 한다.

마지막으로 **제어 흐름**에 관해 **중요한 세부사항 하나**가 더 남아 있다. `.case` 분지 프로세스가 종료하지 않는다면(프로세스 종료에 대해서는 [`chan`식](../chan_expression.md)과 같이 다룬다) `.case` 명령의 닫는 중괄호 다음에 남은 명령을 마저 실행한다. 지역 변수는 모두 유지된다.

## 수신

순서쌍에서 값을 추출할 때는 **수신 명령**을 사용한다.

```par
      strings[str]
```

이 명령을 실행하면 *순서쌍의 왼쪽 원소를 추출해서 `str`에 대입하고, 남은 값은 `strings`의 새로운 값이 된다*.

이제 `Concat`의 골격을 맞춰볼 수 있다.

```par
def Concat = [strings] do {
  let builder = String.Builder
  strings.begin.case {
    .end! => {
      // 아무것도 하지 않음
    }
    .item => {
      strings[str]
      builder.add(str)
      strings.loop
    }
  }
} in builder.build
```

얼핏 보면 *정말* 명령형처럼 생겼다! 변수 `strings`가 프로세스를 거치면서 분기하고, 추출되고, 반복되는 동안 한 번도 재대입이 되지 않았고, 그 대신 자기 자신의 타입이 바뀌는 것을 따라 여러 가지 연산을 수행했다. 그동안 `builder`는 `strings`의 제어 흐름을 따라가면서 결과 값을 한 곳에 모았다.

## `.case` 분지에서의 패턴

`.item =>` 분지를 조금 더 보기 좋게 만들 수 있다. `.begin` 다음의 주어가 [순서쌍](../../types/pair.md)일 경우, `.case` 분지 안에서 바로 패턴 매칭을 할 수 있다.

즉, 다음 코드는...

```par
    .item => {
      strings[str]
      builder.add(str)
      strings.loop
    }
```

다음과 같이 수정할 수 있다.

```par
    .item(str) => {
      builder.add(str)
      strings.loop
    }
```

최종 코드에 반영하면 다음과 같다.

```par
module Main

import {
  @core/List
  @core/String
}

dec Concat : [List<String>] String
def Concat = [strings] do {
  let builder = String.Builder
  strings.begin.case {
    .end! => {}
    .item(str) => {
      builder.add(str)
      strings.loop
    }
  }
} in builder.build

def TestConcat = Concat(*("A", "B", "C"))  // = "ABC"
```

지금까지 살펴본 모든 명령이 조화롭게 한 곳에 모였다.
- [선택](../../types/choice.md) 타입의 **선택**.
- [함수](../../types/function.md) 타입의 **송신**.
- [분기](../../types/either.md) 타입의 **분기**.
- [순서쌍](../../types/pair.md) 타입의 **수신** (여기서는 패턴의 형태로)

이 중 *수신 명령*은 얼핏 쓸모가 적어 보인다. 이제 무한 수열을 다루면서 더욱 흥미로운 용례를 살펴 보자.
