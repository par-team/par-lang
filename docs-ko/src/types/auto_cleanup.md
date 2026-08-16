# 자동 정리

이전 장에서는 [`drop` 제약](./constraints.md#the-drop-constraint)을 소개했다. `drop`을 만족하는 타입의 값은 사용하지 않고 버려도 된다.

`Int`의 경우에는 그렇게 놀랍지 않을 것이다. 하지만 사용하지 않은 값이 열려 있는 파일이나, 스트림이나, 트랜잭션이라면 어떨까? 이들은 선형 값이고, 반대편의 프로세스에서 닫기나 취소, 롤백 신호를 기다리고 있을 수 있기 때문에 단순히 잊어버리고 넘어갈 수는 없다.

그렇다고 해서 항상 모든 것을 일일이 정리해야 한다면 매우 귀찮을 것이다. 오류 경로라면 특히 더 그렇다. 사실 Par에서 자동 정리를 지원하는 이유는 팔할이 [오류 처리](../quality_of_life/error_handling.md)이다. 이 기능이 없으면 모든 오류 경로마다 명시적으로 남은 자원을 모두 정리해야 하고, 심지어 경로마다 남아 있는 자원이 다를 수 있다! 성공 경로에 집중해야 하는데 상용구 코드를 짜는 데 매달릴 수는 없다.

## 별표는 계약이다

선형 자원을 자동 정리하기 위해서는 Par가 그 자원의 프로토콜만 보고 어떻게 정리해야 하는지 알 수 있어야 한다. 여기서는 [선택](./choice.md) 타입에 별표(`*`)로 정리 표지를 추가하면 된다. 정리 표지가 없다면 분지가 하나뿐인 자원이라도 순수 선형으로 남는다.

```par
type StrictResource = choice {
  .release => !,
}

dec IgnoreStrict : [StrictResource] !
def IgnoreStrict = [resource] !  // 오류! `resource`를 사용하지 않음
```

그냥 `.release`를 선택하면 되는 것 아닌가? 그렇게 하지 않는 것은 분지 이름 자체에는 아무런 의미가 없기 때문이다. `.commit`과 `.rollback`이 모두 주어지는 프로토콜이 있다면 컴파일러는 프로그래머가 의도한 분지가 무엇인지 알 수 없다.

타입의 `.release`를 별표로 표시해서 이 분지로 정리해도 안전함을 선언할 수 있다.

```par
type Resource = choice {
  .release* => !,
}

dec Ignore : [Resource] !
def Ignore = [resource] !  // 올바른 코드
```

`resource`가 더 이상 필요하지 않게 되면 Par에서 `.release*`를 알아서 선택한다. 하나의 선택 타입에서는 최대 하나의 분지에만 정리 표지를 붙일 수 있다.

이 별표는 선택 값을 실제로 생성할 때도 등장한다.

```par
def NewResource: Resource = case {
  .release* => !,
}
```

위의 두 표지는 서로 다른 역할을 한다. `Resource` 타입의 별표는 정리 분지가 있다는 약속을 표현하고, 실제로 `Resource` 값을 생성할 때(`case`식)의 별표는 정리 분지를 등록해서 Par의 런타임이 인식해서 정리 시 호출할 수 있도록 한다. 이 점에서 보면 두 번째 별표가 타입에서 요구하는 프로토콜을 만족시키는 것으로 생각할 수 있다.

선택과 분기 타입은 같은 프로토콜의 양면이므로, [분기](./either.md) 타입에도 정리 표지를 추가할 수 있다. 이에 대해서는 [쌍대성](../processes/duality.md)에 대해 배울 때 자세히 다룬다.

## 정리는 타입을 따른다

표시된 분지에서 반환값을 획득한 뒤에도 정리가 계속된다.

```par
type Finalizer = choice {
  .finish* => !,
}

type TwoStageResource = choice {
  .release* => Finalizer,
}

dec IgnoreTwoStage : [TwoStageResource] !
def IgnoreTwoStage = [resource] !
```

`resource`를 정리하면 우선 `.release*`를 선택해 `Finalizer`를 얻는다. Par에서는 이제 이 값에서 다시 `.finish*`를 선택한다. 이 예제에서는 정리 과정이 다음 코드와 같다.

```par
resource.release.finish
```

정리는 **구조적**이다. 즉, Par는 정리의 대상이 되는 값의 형태를 따른다.

- 공유 값은 별도의 작업이 필요 없다.
- 순서쌍은 좌·우의 값을 모두 정리한다.
- 분기는 값에 실제로 존재하는 페이로드를 정리한다.
- 재귀 값은 재귀적으로 정리한다.
- 선택은 정리 표지가 있는 분지를 선택한 뒤 그 결과를 정리한다.
- 반복 값도 정리가 가능하지만, 재귀할 경우 무한루프로 이어질 수 있으므로 재귀적으로 정리하지는 않는다.

이 규칙은 재귀적으로 적용되므로 자원의 리스트라도 통째로 정리할 수 있다.

```par
dec IgnoreAll : [List<Resource>] !
def IgnoreAll = [resources] !
```

Par는 리스트를 순회하면서 안에 있는 모든 자원에 `.release*`를 호출한다.

그러면 선택 값이 `drop`을 만족할 조건은 무엇일까? 다음의 두 조건을 만족해야 한다.

1. 별표로 표시한 분지가 있을 것.
2. 그 분지의 결과 역시 `drop`을 만족할 것.

별표만으로는 충분하지 않으며, 아래의 `IncompleteCleanup`은 `drop`을 만족하지 않는다.

```par
type IncompleteCleanup = choice {
  .release* => [String] !,
}
```

`.release*`를 선택하면 선형 함수를 반환하며, 이 값도 정확히 한 번 호출해야 한다. 별표를 사용해 한 단계의 정리를 수행했지만, 남은 값을 완전히 정리할 수는 없었다.

정리의 결과물에 타입 변수가 있을 경우, 그 변수에 의해 정리가 가능할지가 결정되기도 한다. `.close*` 연산이 실패할 수 있는 쓰기 핸들 타입을 확인해 보자.

```par
type Writer<e> = iterative choice {
  .close* => Try<e, !>,
  .write(Bytes) => Try<e, self>,
}
```

`e`가 `drop`을 만족할 때만 `Writer<e>` 자체가 `drop`을 만족한다. 그렇지 않다면 `.close*`에서 `.err e`를 반환했을 때 이 값을 정리할 수 없는 상황에 처하게 된다. 그러므로 쓰기 핸들을 사용하지 않고 버리는 제네릭 함수는 `e`에 `drop` 제약을 추가해야 한다.

```par
dec AbandonWriter : [type e: drop, Writer<e>] !
def AbandonWriter = [type e: drop, writer] !
```

> 시도 타입은 실패할 수 있는 연산의 결과, 즉 성공이나 오류를 나타내며, 다음과 같이 정의되어 있다.
>
> ```par
> type Try<e, a> = either {
>   .err e,
>   .ok a,
> }
> ```

같은 제약을 사용해 어떤 값에 대해 무슨 타입인지 모르는 채로도 값을 버릴 수 있다.

```par
dec Discard : [<a: drop> a] !
def Discard = [<a: drop> value] !
```

`Discard`는 `a`에 대해 다른 정보 없이도 `drop` 제약 하나만으로 타입 `a`의 어떤 값도 정리가 가능함을 알 수 있다.

Par는 주변의 함수가 완료되는 것을 기다릴 필요 없이 값이 현재 프로세스 경로에서 더 이상 참조되지 않을 때 바로 정리를 시작한다. `.case` 따위의 차단형 연산 이전에 마지막 참조가 쓰였다면, 프로세스가 그 명령에서 기다리기 시작하자마자 정리 역시 시작된다.

약한 선형 값 하나가 이름이 같은 다른 값으로 가려졌을 때도 정리가 일어난다. 순수 선형 값은 이렇게 가릴 수 없으며, 명시적으로 사용해야 한다.

## 자동과 수동 정리의 결정

별표로 표시한 분지도 일반 연산으로 사용할 수 있으며, 직접 선택하는 데 전혀 문제가 없다.

```par
let result = writer.close
```

`.close`를 직접 선택하면 반환된 `result`를 직접 확인할 수 있다. 한편 `writer`를 사용하지 않고 버린다면, Par에서 `.close*`를 선택한 뒤 반환된 시도 값을 정리한다. 이때 닫는 과정에서 `.err`가 반환되더라도 자동으로 정리하기 때문에 오류를 직접 관찰할 수 없다.

이미 다른 오류가 발생해 종료 중이라면, `.close*`에서 발생한 두 번째 오류는 무시하는 것이 프로그래머가 의도한 바일 것이다. 하지만 성공 경로에서는 쓰기 핸들을 닫을 때 버퍼링된 출력을 비우는 과정에서 실패할 수 있다. 이때 직접 `.close`를 호출하고 오류를 전파하면 된다.

```par
writer.close.try
```

여기서 `.try`는 오류 값을 자동 정리에 맡기는 대신 가장 가까운 `catch`에 전파한다. 두 문법에 대해서는 나중에 [오류 처리](../quality_of_life/error_handling.md)에서 자세히 다룬다.

표준 라이브러리의 여러 곳에서도 이 패턴을 사용한다. `Console`, `Bytes.Reader`, `Bytes.Writer`는 `.close*`를, `Stream`은 `.cancel*`을, `Sql.Transaction`은 `.rollback*`을 제공한다.

모든 선택 타입에 정리 분지가 있는 것은 아니다. 정리 표지는 값을 버릴 때 이 분지를 선택하는 것이 항상 안전하다는 의미이며, 그런 성질을 가지는 분지가 없을 때는 표지를 사용하지 않는다. 이때는 선택 값이 순수 선형 값이 되어 명시적으로 사용해야 한다.
