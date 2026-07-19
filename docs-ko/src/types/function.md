# 함수

함수(function)는 인자를 결과값으로 바꾸는 역할을 한다. 함수의 문법은 타입 시스템의 다른 부분과 자연스럽게 연결되도록 특히 신경썼으며, 함수는 [순서쌍](./pair.md)의 [쌍대](../processes/duality.md)이므로 순서쌍의 문법과 매우 유사하다.

함수 타입은 인자와 반환값의 두 타입으로 이루어지며, 전자는 대괄호로 감싸야 한다.

```par
type Function = [Int] String
```

반환값이 다른 함수일 경우, 아래와 같은 *문법 설탕*을 사용해 더 간결하게 작성할 수 있다.

```par
type BinaryFunction1 = [Int] [Int] Int
type BinaryFunction2 = [Int, Int] Int
// 위의 두 타입은 완전히 같음
```

다인자 함수를 정의할 때는 이 방법을 사용하는 것을 권장한다.

**함수는 [선형](../types_and_expressions.md#선형성)이다.** 전역으로 정의한 함수는 임의의 횟수만큼 호출할 수 있지만, 지역 변수에 대입한 함수는 반드시 한 번만 호출해야 한다.

```par
module Main

import @core/Int

dec Add : [Int, Int] Int
def Add = [x, y] x + y

// 전역 함수는 여러 번 호출할 수 있지만...
def Six = Add(1, Add(2, 3))  // 올바른 코드

// 지역 변수에 있는 함수는 한 번만 호출할 수 있음
def Illegal =
  let inc = Add(1)
  in Add(inc(2), inc(3))       // 오류!
```

[선형성](../types_and_expressions.md#선형성)을 채택한 덕에 선형 시스템만의 강력한 표현력을 구현할 수 있다. 선형 타입에서 비롯되는 새로운 패러다임으로 어디까지 갈 수 있을지 확인하는 것이 바로 *Par*의 주 목적이다.

> 비선형 함수는 [박스 타입](./box.md)으로써 구현할 수 있다.

## 생성

함수 값은 우선 대괄호 안에 입력받을 인자를 작성하고, 그 뒤에 그 인자를 사용해 결과를 계산하는 식을 작성해서 생성할 수 있다.

```par
dec Double : [Int] Int
def Double = [number] 2 * number
```

다인자 함수(엄밀하게는 다른 함수를 반환하는 함수)는 타입 문법에서 지원하는 문법 설탕을 동일하게 사용해서 작성할 수 있다.

```par
module Main

import @core/String

dec Concat : [String, String] String
// `[String] [String] String`과 같음

def Concat = [left, right]
  String.Builder.add(left).add(right).build
```

대괄호 안의 패턴에서 [순서쌍](./pair.md)과 [단위](./unit.md) 타입을 매치하는 문법 역시 사용할 수 있다.

```par
dec Swap : [(String, Int)!] (Int, String)!
def Swap = [(x, y)!] (y, x)!
```


Par는 [양방향 타입 검사](https://arxiv.org/abs/1908.05839)를 사용한다. 이런 스타일의 타입 검사로 여러 가지 타입을 추론할 수 있지만, 불필요하게 추측하려는 시도는 하지 않는다. 이 방법으로 완전히 추론할 수 없는 타입 중 하나가 바로 함수이다.

```par
def Identity = [x] x  // 오류! `x`의 타입을 알 수 없음
```

함수의 타입을 미리 알 수 없다면, 적어도 함수의 인자는 명시적으로 타입 표기를 해야 한다.

```par
def Identity = [x: String] x  // 올바른 코드
```

**제네릭 함수**에 대해서는 [전칭 타입](./forall.md)을 참고하라.

> Par는 [전체성](../introduction.md#전체성을-향한-야심찬-도전)이라는 야심찬 목표를 달성하기 위해 자기 참조에 의한 **초보적 재귀가 불가하다는 다소 특이한 결정**을 내렸다. 즉, **함수가 자기 자신을 직접 호출할 수 없다.**
> 
> ```par
> def Infinity = 1 + Infinity  // 오류! 순환 참조
> ```
> 
> 그 대신 재귀와 [쌍대재귀](https://en.wikipedia.org/wiki/Corecursion)를 구현하기 위해 각각 [재귀](./recursive.md) 타입과 [반복](./iterative.md) 타입을 사용한다. 이 타입들 역시 더 나중에 다룰 예정이다.
> 
> Par에서는 `begin`/`loop`이라는 강력한 보편 구조로 순환 연산을 구현할 수 있으며, 재귀함수와 [반복](./iterative.md) 개체, [프로세스 문법](../process_syntax.md)의 명령형처럼 보이는 반복문도 이 하나의 문법으로 구현할 수 있다.
> 
> 함수의 재귀 호출이 불가하다는 것은 커다란 제약처럼 느껴질 수 있지만, 지역 변수를 자연스럽게 다룰 수 있고 헬퍼 함수 없이 식의 깊은 곳에도 사용할 수 있는 `begin`/`loop` 문법으로 완전히 해소할 수 있다.

## 소멸

함수 호출은 익숙한 문법으로 할 수 있다.

```par
def Ten = Double(5)  // 위에서 정의한 `Double`을 사용
```

다인자 함수를 호출할 때는 소괄호 안에 인자를 반점으로 구분해 작성할 수도 있다.

```par
def HelloWorld1 = Concat("Hello ", "World")  // 위에서 정의한 `Concat`을 사용
def HelloWorld2 = Concat("Hello ")("World")

def HelloWorld3 =
  let partial = Concat("Hello ")
  in partial("World")
```

위의 세 문법 모두 같은 동작을 한다.

함수의 경우에는 [선형성](../types_and_expressions.md#선형성)에 의해 *소멸*이라는 단어 선택이 특히 적절하다. 지역 변수에 대입된 함수를 호출하면 위에서 언급한 것과 같이 그 변수가 소멸한다.
